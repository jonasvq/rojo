local getProperty = require(script.Parent.getProperty)
local decodeValue = require(script.Parent.decodeValue)
local trueEquals = require(script.Parent.trueEquals)
local Packages = script.Parent.Parent.Parent.Packages
local RbxDom = require(Packages.RbxDom)
local Log = require(Packages.Log)

-- Planning must finish without yielding or mutating Studio. Ambiguity is a
-- conflict, never permission to take an arbitrary same-named object.
local function plan(instanceMap, patch)
	local budget, buckets, planned, reserved, visited = 200000, {}, {}, {}, {}
	local opaqueCount, opaqueNames = 0, {}
	local function inspect()
		budget -= 1
		assert(budget >= 0, "Adoption inspection budget exceeded; split the saved batch")
	end
	local function fail(record, reason)
		error(
			("Cannot adopt %s (%s): %s. Rename/remove the conflict or reconnect and review initial sync."):format(
				record.Name,
				record.ClassName,
				reason
			),
			0
		)
	end
	local removed, replaced = {}, {}
	for _, id in patch.removed do
		removed[instanceMap.fromIds[id] or id] = true
	end
	for _, update in patch.updated do
		if update.changedClassName and instanceMap.fromIds[update.id] then
			replaced[instanceMap.fromIds[update.id]] = true
		end
	end
	local function survives(instance)
		local ancestor = instance
		while ancestor do
			inspect()
			if removed[ancestor] or replaced[ancestor] then
				return false
			end
			ancestor = ancestor.Parent
		end
		return true
	end
	local function children(parent)
		if not buckets[parent] then
			local names = {}
			for _, child in parent:GetChildren() do
				inspect()
				local name = child.Name
				names[name] = names[name] or {}
				table.insert(names[name], child)
			end
			buckets[parent] = names
		end
		return buckets[parent]
	end
	local function visit(id, parent, depth, required)
		inspect()
		assert(depth <= 128 and not visited[id], "Invalid or excessively deep adoption forest")
		visited[id] = true
		local record = assert(patch.added[id], "Missing adoption child")
		local matches = parent and children(parent)[record.Name]
		local candidate
		if matches then
			if #matches ~= 1 then
				fail(record, "multiple same-name Studio candidates")
			end
			candidate = matches[1]
			if candidate.ClassName ~= record.ClassName or not candidate.Archivable then
				fail(record, "class differs or candidate cannot be saved")
			end
			if instanceMap.fromInstances[candidate] or reserved[candidate] then
				fail(record, "candidate already belongs to another managed ID")
			end
			if not survives(candidate) then
				fail(record, "candidate or ancestor is being removed/replaced")
			end
			planned[id], reserved[candidate] = candidate, true
		elseif required then
			fail(record, "saved subtree has no matching Studio child")
		end
		for _, childId in record.Children do
			local child = assert(patch.added[childId], "Missing adoption child")
			assert(child.Parent == id, "Inconsistent adoption parent")
			visit(childId, candidate, depth + 1, candidate ~= nil)
		end
	end
	local roots = {}
	for id, record in patch.added do
		inspect()
		if not patch.added[record.Parent] and not instanceMap.fromIds[id] then
			table.insert(roots, id)
		end
	end
	table.sort(roots)
	for _, id in roots do
		local parent = assert(instanceMap.fromIds[patch.added[id].Parent], "Missing adoption parent")
		visit(id, parent, 1, false)
	end
	for id in patch.added do
		assert(visited[id] or instanceMap.fromIds[id], "Cyclic or disconnected adoption forest")
	end
	local tentative = { fromIds = setmetatable(planned, { __index = instanceMap.fromIds }) }
	for id, instance in planned do
		local record = patch.added[id]
		for property, value in record.Properties do
			inspect()
			local descriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, property)
			if descriptor and descriptor.scriptability == "None" then
				-- Live sync cannot read or assign these fields. Opt-in adoption
				-- preserves Studio's values; it does not assert binary equality.
				opaqueCount += 1
				if #opaqueNames < 8 then
					table.insert(opaqueNames, instance.ClassName .. "." .. property)
				end
				continue
			end
			local read, existing = getProperty(instance, property)
			local decoded, expected = decodeValue(value, tentative)
			if not read or not decoded or not trueEquals(existing, expected) then
				fail(record, "declared property " .. property .. " differs or cannot be verified")
			end
			if typeof(expected) == "Instance" and not survives(expected) then
				fail(record, "reference target is being removed/replaced")
			end
		end
	end
	-- Do not leak the fallback metatable into the commit iteration.
	setmetatable(planned, nil)
	return planned, opaqueCount, opaqueNames
end

return function(instanceMap, patch)
	local ok, planned, opaqueCount, opaqueNames = pcall(plan, instanceMap, patch)
	if not ok then
		return false, tostring(planned)
	end
	local inserted = {}
	local committed, reason = pcall(function()
		for id, instance in planned do
			table.insert(inserted, id)
			instanceMap:insert(id, instance)
		end
	end)
	if not committed then
		for _, id in inserted do
			instanceMap:removeId(id)
		end
		return false, "Adoption mapping failed before patch application: " .. tostring(reason)
	end
	if opaqueCount > 0 then
		Log.warn(
			"Adopted saved Studio instances using readable live properties. {} serialization-only values were preserved in Studio and remain unverified against the file ({}). Build output may differ.",
			opaqueCount,
			table.concat(opaqueNames, ", ")
		)
	end
	return true
end
