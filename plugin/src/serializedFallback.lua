local SerializationService = game:GetService("SerializationService")
local Log = require(script.Parent.Parent.Packages.Log)
local PatchSet = require(script.Parent.PatchSet)

local function decode(id, contents)
	if typeof(contents) ~= "buffer" then
		return nil
	end
	local ok, objects = pcall(SerializationService.DeserializeInstancesAsync, SerializationService, contents)
	if not ok then
		Log.warn("Could not deserialize fallback model {}: {}", id, objects)
		return nil
	end
	local valid, value = pcall(function()
		local root = objects[1]
		local item = root and root:GetChildren()[1]
		if
			#objects == 1
			and root:IsA("Folder")
			and #root:GetChildren() == 1
			and item:IsA("ObjectValue")
			and item.Name == id
			and item.Value
			and item.Value:IsDescendantOf(item)
		then
			local replacement = item.Value
			replacement.Parent = nil
			return replacement
		end
		return nil
	end)
	for _, object in objects do
		object:Destroy()
	end
	return if valid then value else nil
end

-- Serial legacy requests intentionally bound both HTTP and native deserializer
-- concurrency. A single broken model must not reject the other roots.
return function(api, ids)
	local replacements = {}
	local function discard()
		for _, value in replacements do
			value:Destroy()
		end
		return {}, nil
	end
	local refPatch
	if api.__serializedModels then
		local ok, response = api:serialize(ids):await()
		if
			not ok
			or type(response) ~= "table"
			or type(response.models) ~= "table"
			or not PatchSet.validate(response.refPatch)
		then
			return discard()
		end
		refPatch = response.refPatch
		local wanted, seen = {}, {}
		for _, id in ids do
			wanted[id] = true
		end
		for _, entry in response.models do
			if type(entry) ~= "table" or type(entry.id) ~= "string" or not wanted[entry.id] then
				return discard()
			end
			local id = entry.id
			if seen[id] then
				if replacements[id] then
					replacements[id]:Destroy()
					replacements[id] = nil
				end
				seen[id] = "duplicate"
			elseif entry.error == nil then
				seen[id] = true
				replacements[id] = decode(id, entry.modelContents)
			else
				seen[id] = true
				Log.warn("Could not serialize fallback model {}: {}", id, entry.error)
			end
		end
	else
		local seen = {}
		for _, id in ids do
			if seen[id] then
				continue
			end
			seen[id] = true
			local ok, response = api:serialize({ id }):await()
			if ok and type(response) == "table" then
				replacements[id] = decode(id, response.modelContents)
			end
		end
		local successful = {}
		for id in replacements do
			table.insert(successful, id)
		end
		if #successful == 0 then
			return discard()
		end
		local ok, response = api:refPatch(successful):await()
		if not ok or type(response) ~= "table" or not PatchSet.validate(response.patch) then
			return discard()
		end
		refPatch = response.patch
		local readOk, read = api:read(successful):await()
		if not readOk or type(read) ~= "table" or type(read.instances) ~= "table" then
			return discard()
		end
		for _, id in successful do
			local instance = read.instances[id]
			if type(instance) ~= "table" or type(instance.Properties) ~= "table" then
				return discard()
			end
			local properties = {}
			for name, value in instance.Properties do
				if type(value) == "table" and value.Ref ~= nil then
					properties[name] = value
				end
			end
			table.insert(refPatch.updated, { id = id, changedProperties = properties })
		end
	end
	return replacements, refPatch
end
