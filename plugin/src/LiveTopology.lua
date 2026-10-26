local InstanceMap = require(script.Parent.InstanceMap)
local PatchSet = require(script.Parent.PatchSet)
local adoptAdded = require(script.Parent.Reconciler.adoptAdded)
local reify = require(script.Parent.Reconciler.reify)
local getProperty = require(script.Parent.Reconciler.getProperty)
local setProperty = require(script.Parent.Reconciler.setProperty)
local decodeValue = require(script.Parent.Reconciler.decodeValue)
local encodePatchUpdate = require(script.Parent.ChangeBatcher.encodePatchUpdate)

local LiveTopology = {}
LiveTopology.__index = LiveTopology

local function bounded(value, limit)
	local text = tostring(value)
	local length = math.min(#text, limit or 512)
	while length > 0 and utf8.len(text:sub(1, length)) == nil do
		length -= 1
	end
	return text:sub(1, length)
end

local function copy(value, depth)
	if type(value) ~= "table" then
		return value
	end
	assert((depth or 0) < 32, "Topology property exceeds supported wire depth")
	local result = {}
	for key, child in value do
		result[key] = copy(child, (depth or 0) + 1)
	end
	return result
end

function LiveTopology.new(map)
	return setmetatable(
		{ map = map, records = {}, dirty = {}, appliedChanges = {}, version = 0, epoch = 0, active = true },
		LiveTopology
	)
end

function LiveTopology:seed(records, root)
	self.records = copy(records)
	for id, record in self.records do
		record.Id = id
	end
	self.root = root
	self.rootInstance = self.map.fromIds[root]
	self.version += 1
	table.clear(self.dirty)
end

function LiveTopology:mark(id)
	if self.committing then
		return
	end
	if id and id ~= self.root and self.records[id] then
		self.dirty[id] = true
		self.epoch += 1
	end
end

function LiveTopology:stop()
	self.active = false
	self.version += 1
	table.clear(self.records)
	table.clear(self.dirty)
	table.clear(self.appliedChanges)
	self.rootInstance = nil
end

function LiveTopology:recordAppliedChange(id, property)
	local record = id and self.records[id]
	if record and property ~= "Source" and record.Properties[property] ~= nil then
		self.appliedChanges[id] = self.appliedChanges[id] or {}
		self.appliedChanges[id][property] = true
	end
end

function LiveTopology:accept(patch, unapplied)
	if not self.active or not self.root then
		return
	end
	local records = self.records
	local removed, parents = {}, {}
	local function remove(id)
		local pending = { id }
		while #pending > 0 do
			local current = table.remove(pending)
			if removed[current] then
				continue
			end
			removed[current] = true
			local record = records[current]
			if record then
				if record.Parent then
					parents[record.Parent] = true
				end
				for _, child in record.Children do
					table.insert(pending, child)
				end
			end
			records[current], self.dirty[current] = nil, nil
		end
	end
	for _, id in patch.removed do
		remove(id)
	end
	for id, record in patch.added do
		records[id] = copy(record)
		records[id].Id = id
	end
	for _, update in patch.updated do
		local record = records[update.id]
		if not record then
			continue
		end
		if update.changedName then
			record.Name = update.changedName
		end
		if update.changedClassName then
			record.ClassName = update.changedClassName
		end
		if update.changedMetadata then
			record.Metadata = copy(update.changedMetadata)
		end
		for key, value in update.changedProperties or {} do
			record.Properties[key] = copy(value)
		end
		for _, key in update.removedProperties or {} do
			local instance = self.map.fromIds[update.id]
			local encoded = instance and encodePatchUpdate(instance, update.id, { [key] = true }, self.map)
			record.Properties[key] = encoded and encoded.changedProperties[key] or nil
			if not record.Properties[key] then
				record.repairUnavailable = true
			end
		end
	end
	-- Rebuild only changed child lists. Server updates do not otherwise carry
	-- Parent, so root/descendant relationships stay tied to the accepted IDs.
	for _, record in patch.added do
		if record.Parent and records[record.Parent] and not patch.added[record.Parent] then
			parents[record.Parent] = true
		end
	end
	for parent in parents do
		if not records[parent] then
			continue
		end
		local children, seen = {}, {}
		for _, id in records[parent].Children do
			if records[id] and not seen[id] then
				table.insert(children, id)
				seen[id] = true
			end
		end
		for id, record in patch.added do
			if record.Parent == parent and not seen[id] then
				table.insert(children, id)
				seen[id] = true
			end
		end
		records[parent].Children = children
	end
	for id in (unapplied and unapplied.added or {}) do
		if records[id] then
			records[id].repairUnavailable = true
		end
	end
	for _, update in (unapplied and unapplied.updated or {}) do
		if records[update.id] then
			records[update.id].repairUnavailable = true
		end
	end
	-- Engine setters can update a descendant's declared fields (ScaleTo changes
	-- Size/CFrame). Keep that accepted live baseline without inventing file data.
	for id, properties in self.appliedChanges do
		local instance, record = self.map.fromIds[id], records[id]
		if not instance or not record then
			continue
		end
		local encoded = encodePatchUpdate(instance, id, properties, self.map)
		for property in properties do
			local value = encoded and encoded.changedProperties[property]
			if value then
				record.Properties[property] = copy(value)
			else
				record.repairUnavailable = true
			end
		end
	end
	table.clear(self.appliedChanges)
	self.version += 1
end

function LiveTopology:check(patch, adopt, getProtection, beginMutation)
	if not self.active or not self.root then
		return true
	end
	if self.checking then
		return false, "Topology recovery is already in progress; reconnect and review"
	end
	self.checking = true
	local version, epoch, map, records = self.version, self.epoch, self.map, self.records
	local shadow, staging
	local finish, committed = nil, false
	local reviewRoots = {}
	local moves, rebound, oldMappings, referenceChanges, paused, created = {}, {}, {}, {}, {}, {}
	local function pause(instance)
		if paused[instance] == nil then
			paused[instance] = map.pausedUpdateInstances[instance] == true
			map:pauseInstance(instance)
		end
	end
	local function fail(id, reason)
		local record = records[id]
		error(
			("Cannot recover managed topology for %s (%s): %s. Local objects were retained; reconnect and review."):format(
				bounded(record and record.Name or id, 128),
				bounded(id, 64),
				reason
			),
			0
		)
	end
	local ok, reason = pcall(function()
		local budget, checked, broken, depths, removing = 200000, {}, {}, {}, {}
		local function inspect()
			budget -= 1
			assert(budget >= 0, "Topology recovery inspection budget exceeded")
		end
		local function isRemoving(id)
			local path, visiting = {}, {}
			local current = id
			while current and removing[current] == nil do
				inspect()
				assert(#path < 128 and not visiting[current], "Cyclic or excessively deep accepted topology")
				local record = records[current]
				if not record or current == self.root then
					break
				end
				visiting[current] = true
				table.insert(path, current)
				current = record.Parent
			end
			local value = current and removing[current] or false
			for _, child in path do
				removing[child] = value
			end
			return value
		end
		for _, id in patch.removed do
			removing[id] = true
		end
		local function visit(id, depth)
			inspect()
			assert(depth < 128, "Accepted topology is too deep to repair")
			if checked[id] or isRemoving(id) then
				return
			end
			local record = records[id]
			if not record then
				return
			end
			checked[id] = true
			if id == self.root then
				if map.fromIds[id] ~= self.rootInstance then
					fail(id, "accepted root namespace changed")
				end
				depths[id] = 0
				return
			end
			if not record.Parent or not records[record.Parent] then
				fail(id, "accepted parent is unavailable")
			end
			visit(record.Parent, depth + 1)
			depths[id] = (depths[record.Parent] or 0) + 1
			local instance, parent = map.fromIds[id], map.fromIds[record.Parent]
			if not instance or not parent or instance.Parent ~= parent or instance.Name ~= record.Name then
				broken[id] = true
			end
		end
		for id in self.dirty do
			visit(id, 0)
		end
		for _, update in patch.updated do
			visit(update.id, 0)
		end
		for _, record in patch.added do
			if record.Parent then
				visit(record.Parent, 0)
			end
		end
		local ordered = {}
		for id in broken do
			table.insert(ordered, id)
		end
		table.sort(ordered, function(a, b)
			return if depths[a] == depths[b] then a < b else depths[a] < depths[b]
		end)
		if #ordered == 0 then
			table.clear(self.dirty)
			return
		end
		shadow, staging = InstanceMap.new(), Instance.new("Folder")
		for id, instance in map.fromIds do
			shadow.fromIds[id], shadow.fromInstances[instance] = instance, id
		end
		local protected = getProtection()
		local covered, touched, candidatesById, additions, deferredRefs = {}, {}, {}, PatchSet.newEmpty(), {}
		local function retainReview(candidates)
			for _, candidate in candidates or {} do
				reviewRoots[candidate] = true
			end
		end
		local function subtree(id, output, depth, requireComplete, candidates)
			inspect()
			assert(depth < 128 and not output[id], "Invalid recovery subtree")
			local record = records[id]
			if not record or (requireComplete and record.repairUnavailable) then
				fail(id, "no complete accepted baseline is available")
			end
			if protected[id] then
				retainReview(candidates)
				fail(id, "protected Studio properties or unconfirmed local Source must be reviewed first")
			end
			if requireComplete and (record.Properties.Parent or record.Properties.Name) then
				fail(id, "structural wire properties are not reversible")
			end
			output[id] = record
			touched[id] = true
			candidatesById[id] = candidates
			for _, child in record.Children do
				subtree(child, output, depth + 1, requireComplete, candidates)
			end
		end
		for _, id in ordered do
			if covered[id] then
				continue
			end
			local record, instance = records[id], map.fromIds[id]
			local parent = map.fromIds[record.Parent]
			if not parent then
				fail(id, "mapped parent is missing")
			end
			local forest = {}
			local candidates = {}
			for _, child in parent:GetChildren() do
				inspect()
				if child ~= instance and child.Name == record.Name then
					table.insert(candidates, child)
				end
			end
			local live = instance and map:hasLiveSignals(instance)
			subtree(id, forest, 0, #candidates > 0 or not live, candidates)
			if #candidates > 0 then
				if instance and instance.Parent ~= nil then
					fail(id, "a competing object occupies the original slot")
				end
				if not adopt then
					fail(id, "a replacement needs the opt-in saved-instance adoption setting")
				end
				for child, childRecord in forest do
					covered[child], additions.added[child] = true, childRecord
					shadow:removeId(child)
				end
			else
				-- Destroy disconnects an instance's signals; assigning nil to its
				-- already-nil locked Parent can misleadingly succeed.
				if live then
					table.insert(moves, {
						instance = instance,
						parent = parent,
						name = record.Name,
						oldParent = instance.Parent,
						oldName = instance.Name,
					})
				else
					for child in forest do
						local old = map.fromIds[child]
						if old and old.Parent ~= nil and child ~= id then
							fail(id, "a live descendant exists outside the deleted subtree")
						end
						covered[child] = true
						shadow:removeId(child)
					end
					local refs = {}
					local unapplied = reify.reifyInstance(refs, shadow, forest, id, staging, function(allocated)
						table.insert(created, allocated)
					end)
					if not PatchSet.isEmpty(unapplied) then
						fail(id, "the accepted subtree cannot be reconstructed faithfully")
					end
					for _, ref in refs do
						table.insert(deferredRefs, ref)
					end
					local replacement = shadow.fromIds[id]
					table.insert(moves, {
						instance = replacement,
						parent = parent,
						name = record.Name,
						oldParent = staging,
						oldName = record.Name,
					})
					for child in forest do
						rebound[child] = shadow.fromIds[child]
					end
				end
			end
		end
		if next(additions.added) then
			local adopted, adoptionReason = adoptAdded(shadow, additions)
			if not adopted then
				error(adoptionReason, 0)
			end
			for id in additions.added do
				rebound[id] = shadow.fromIds[id]
			end
		end
		local failedRefs = PatchSet.newEmpty()
		reify.applyDeferredRefs(shadow, deferredRefs, failedRefs)
		if not PatchSet.isEmpty(failedRefs) then
			error("Topology recovery references could not be reconstructed", 0)
		end
		-- Repair references from survivors too; a fresh mapping must not leave
		-- an ObjectValue/PrimaryPart pointing at the detached old target.
		for id, record in records do
			inspect()
			local instance = shadow.fromIds[id]
			if not instance or isRemoving(id) then
				continue
			end
			for property, value in record.Properties do
				if type(value) == "table" and type(value.Content) == "table" and rebound[value.Content.Object] then
					fail(id, "an affected Content.Object reference cannot be rebound by the current live codec")
				end
				if type(value) == "table" and value.Ref and rebound[value.Ref] then
					local read, old = getProperty(instance, property)
					local decoded, target = decodeValue(value, shadow)
					if not read or not decoded then
						fail(id, "an affected reference cannot be verified")
					end
					table.insert(
						referenceChanges,
						{ instance = instance, property = property, old = old, value = target }
					)
				end
			end
		end
		if not self.active or self.version ~= version or self.epoch ~= epoch then
			error("Topology baseline changed during recovery preparation", 0)
		end
		local latestProtection = getProtection()
		for id in touched do
			if latestProtection[id] then
				retainReview(candidatesById[id])
				fail(id, "new local Source edits appeared during recovery")
			end
		end
		finish = beginMutation()
		assert(finish, "Cannot safely repair topology while another history recording is active")
		self.committing = true
		for _, move in moves do
			pause(move.instance)
			move.instance.Name = move.name
			if move.instance.Parent ~= move.parent then
				move.instance.Parent = move.parent
			end
		end
		for id, replacement in rebound do
			oldMappings[id] = { instance = map.fromIds[id] }
			map:insert(id, replacement)
		end
		for _, change in referenceChanges do
			pause(change.instance)
			local assigned, assignmentReason = setProperty(change.instance, change.property, change.value)
			assert(assigned, tostring(assignmentReason))
		end
		committed = true
		table.clear(self.dirty)
	end)
	if not ok and finish then
		local rollbackErrors = {}
		local function restore(label, callback)
			local restored, restoreReason = pcall(callback)
			if not restored and #rollbackErrors < 3 then
				table.insert(rollbackErrors, label .. ": " .. tostring(restoreReason):sub(1, 300))
			end
		end
		for index = #referenceChanges, 1, -1 do
			local change = referenceChanges[index]
			restore("reference " .. change.property, function()
				local assigned, assignReason = setProperty(change.instance, change.property, change.old)
				assert(assigned, tostring(assignReason))
			end)
		end
		for id, old in oldMappings do
			restore("mapping " .. id, function()
				if old.instance then
					map:insert(id, old.instance)
				else
					map:removeId(id)
				end
			end)
		end
		for index = #moves, 1, -1 do
			local move = moves[index]
			restore("parent/name " .. move.oldName, function()
				move.instance.Parent = move.oldParent
				move.instance.Name = move.oldName
			end)
		end
		if #rollbackErrors > 0 then
			reason = "Recovery rollback was incomplete; review local state: "
				.. bounded(reason, 256)
				.. "; "
				.. table.concat(rollbackErrors, "; ")
		end
	end
	for instance, wasPaused in paused do
		if not wasPaused then
			map:unpauseInstance(instance)
		end
	end
	if shadow then
		shadow:stop()
	end
	if staging then
		staging:Destroy()
	end
	if not committed then
		for _, instance in created do
			instance:Destroy()
		end
	end
	if finish then
		finish(committed)
	end
	self.checking = false
	self.committing = false
	return ok, if ok then nil else bounded(reason), reviewRoots
end

return LiveTopology
