-- Class-specific source of truth. This module grants no filesystem writes.
local ClassAuthority = {}
ClassAuthority.__index = ClassAuthority

-- Preserve the need for review across reconnects without retaining instances.
local held = setmetatable({}, { __mode = "k" })
local pending = setmetatable({}, { __mode = "k" })
local pendingContext

function ClassAuthority.new(map)
	return setmetatable({ map = map, modes = {}, preserve = {}, negotiated = false }, ClassAuthority)
end

local function checked(mode)
	assert(
		mode == "legacy" or mode == "filesystem" or mode == "studio",
		"Unsupported or missing class authority; reconnect with a matching server/plugin"
	)
	return mode
end

function ClassAuthority:seed(records, modes, negotiated, context)
	self.negotiated = negotiated
	self.context = context
	self.initializing = true
	self.review = pendingContext ~= nil
		and context ~= nil
		and pendingContext.sessionId == context.sessionId
		and pendingContext.root == context.root
		and pendingContext.rootId == context.rootId
	self.modes = {}
	for id in records do
		self.modes[id] = if negotiated then checked(modes and modes[id]) else "legacy"
	end
end

function ClassAuthority:requireReview()
	if self.context and next(self:protectedIds()) then
		pendingContext = self.context
	end
end

function ClassAuthority:explicit(id)
	return self.modes[id] ~= nil and self.modes[id] ~= "legacy"
end

function ClassAuthority:validateHydration(records, root, instance)
	if next(self:protectedIds()) == nil then
		return
	end
	local protected = {}
	local visiting = {}
	local function needsUnique(id, depth)
		assert(depth < 128 and not visiting[id], "Invalid class-authority tree")
		if protected[id] ~= nil then
			return protected[id]
		end
		visiting[id] = true
		local required = self.modes[id] == "studio"
		for _, child in records[id].Children do
			required = needsUnique(child, depth + 1) or required
		end
		visiting[id] = nil
		protected[id] = required
		return required
	end
	needsUnique(root, 0)
	local work = { { root, instance } }
	while #work > 0 do
		local pair = table.remove(work)
		local buckets, virtualCounts = {}, {}
		for _, child in pair[2]:GetChildren() do
			local ok, name, class = pcall(function()
				return child.Name, child.ClassName
			end)
			if ok then
				buckets[name] = buckets[name] or {}
				buckets[name][class] = buckets[name][class] or {}
				table.insert(buckets[name][class], child)
			end
		end
		for _, id in records[pair[1]].Children do
			local node = records[id]
			virtualCounts[node.Name] = virtualCounts[node.Name] or {}
			virtualCounts[node.Name][node.ClassName] = (virtualCounts[node.Name][node.ClassName] or 0) + 1
		end
		for _, id in records[pair[1]].Children do
			if not protected[id] then
				continue
			end
			local node = records[id]
			local candidates = buckets[node.Name] and buckets[node.Name][node.ClassName] or {}
			assert(
				#candidates <= 1 and virtualCounts[node.Name][node.ClassName] == 1,
				"Studio class authority requires unambiguous instance identity; duplicated names were retained without syncing"
			)
			if #candidates == 1 then
				table.insert(work, { id, candidates[1] })
			end
		end
	end
end

function ClassAuthority:containsHeld(instance)
	if held[instance] or self.preserve[self.map.fromInstances[instance]] then
		return true
	end
	for _, child in instance:GetDescendants() do
		if held[child] or self.preserve[self.map.fromInstances[child]] then
			return true
		end
	end
	return false
end

function ClassAuthority:protectedIds()
	local result = {}
	for id, mode in self.modes do
		if mode == "studio" then
			result[id] = true
		end
	end
	return result
end

function ClassAuthority:initial(patch)
	local review = self.review or false
	for id, instance in self.map.fromIds do
		if self.modes[id] == "studio" then
			self.preserve[id] = true
		end
		if pending[instance] or (held[instance] and self.modes[id] ~= "studio") then
			review = true
		end
	end
	for _, removed in patch.removed do
		local instance = if typeof(removed) == "Instance" then removed else self.map.fromIds[removed]
		if instance and self:containsHeld(instance) then
			review = true
		end
	end
	for _, update in patch.updated do
		local instance = self.map.fromIds[update.id]
		if
			instance
			and (
				(held[instance] and self.modes[update.id] ~= "studio")
				or (update.changedClassName and self:containsHeld(instance))
			)
		then
			review = true
		end
	end
	return self:filter(patch), review
end

function ClassAuthority:filter(patch)
	local filtered = table.clone(patch)
	filtered.updated = {}
	for _, update in patch.updated do
		if self.modes[update.id] == "studio" and self.map.fromIds[update.id] and not update.changedClassName then
			update = table.clone(update)
			update.changedProperties = {}
			update.removedProperties = nil
		end
		local instance = self.map.fromIds[update.id]
		if
			instance
			and instance:IsA("Model")
			and (
				(update.changedProperties and update.changedProperties.Scale)
				or table.find(update.removedProperties or {}, "Scale")
			)
		then
			assert(
				not self:containsHeld(instance),
				"Filesystem Model.Scale would change Studio-owned descendants; keep model scale Studio-owned or export the model explicitly"
			)
		end
		table.insert(filtered.updated, update)
	end
	return filtered
end

function ClassAuthority:prepare(patch)
	local modes = patch.classAuthority or {}
	for id in patch.added do
		if self.negotiated then
			checked(modes[id])
		end
	end
	for _, update in patch.updated do
		local mode = if self.negotiated then checked(modes[update.id]) else "legacy"
		local old = self.modes[update.id] or "legacy"
		if mode ~= old and (mode == "studio" or old == "studio") then
			local instance = self.map.fromIds[update.id]
			if instance then
				pending[instance] = true
			end
			return nil,
				"Class authority changed; current Studio values were retained. Reconnect and explicitly review the new policy."
		end
		local instance = self.map.fromIds[update.id]
		if update.changedClassName and instance and self:containsHeld(instance) then
			return nil, "Class replacement would discard Studio-owned properties; reconnect and review."
		end
	end
	for _, id in patch.removed do
		local instance = if typeof(id) == "Instance" then id else self.map.fromIds[id]
		if instance and self:containsHeld(instance) then
			return nil, "Removal would discard Studio-owned properties; reconnect and review."
		end
	end
	return self:filter(patch)
end

function ClassAuthority:accept(patch)
	for id in patch.added do
		self.modes[id] = if self.negotiated
			then checked(patch.classAuthority and patch.classAuthority[id])
			else "legacy"
	end
	for _, update in patch.updated do
		if patch.classAuthority then
			self.modes[update.id] = checked(patch.classAuthority[update.id])
		end
	end
	local function remember(id)
		local instance = self.map.fromIds[id]
		if instance then
			held[instance] = if self.modes[id] == "studio" then true else nil
			pending[instance] = nil
		end
	end
	if self.initializing then
		for id in self.map.fromIds do
			remember(id)
		end
		self.initializing = false
		table.clear(self.preserve)
	else
		for id in patch.added do
			remember(id)
		end
		for _, update in patch.updated do
			remember(update.id)
		end
	end
	if #patch.removed > 0 then
		for id in self.modes do
			if not self.map.fromIds[id] then
				self.modes[id] = nil
			end
		end
	end
	if self.review then
		pendingContext, self.review = nil, false
	end
end

function ClassAuthority:stop()
	table.clear(self.modes)
	table.clear(self.preserve)
end

return ClassAuthority
