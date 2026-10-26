return function()
	local warning = require(script.Parent.unknownRemovalWarning)
	it("counts unknown roots and descendants without managed IDs or overlapping roots", function()
		local root = Instance.new("Folder")
		root.Name = "StudioOnly"
		local child = Instance.new("Folder")
		child.Name = "Grandchild"
		child.Parent = root
		local message = warning({ removed = { "MANAGED", root, child, root } })
		root:Destroy()
		assert(string.find(message, "1 unknown Studio roots and 1 descendants", 1, true))
		assert(string.find(message, "StudioOnly", 1, true))
		assert(string.find(message, "$ignoreUnknownInstances", 1, true))
		assert(string.find(message, "$ignoreUnknownDescendants", 1, true))
		assert(warning({ removed = { "MANAGED" } }) == nil, "managed deletions generated an unknown-instance warning")
	end)
	it("bounds path samples and long names", function()
		local instances = {}
		for _ = 1, 15 do
			local instance = Instance.new("Folder")
			instance.Name = string.rep("x", 300)
			table.insert(instances, instance)
		end
		local message = warning({ removed = instances })
		for _, instance in instances do
			instance:Destroy()
		end
		assert(string.find(message, "15 unknown Studio roots and 0 descendants", 1, true))
		assert(#message < 2400, "warning paths were not bounded")
	end)
end
