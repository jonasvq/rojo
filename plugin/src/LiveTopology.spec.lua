return function()
	local LiveTopology = require(script.Parent.LiveTopology)
	local InstanceMap = require(script.Parent.InstanceMap)
	local PatchSet = require(script.Parent.PatchSet)
	local reify = require(script.Parent.Reconciler.reify)
	local function fixture(callback)
		local root = Instance.new("Folder")
		local target = Instance.new("Part")
		target.Name, target.Anchored, target.Parent = "Target", true, root
		target:SetAttribute("Owner", "base")
		target:AddTag("Managed")
		local source = Instance.new("ModuleScript")
		source.Name, source.Source, source.Parent = "Child", "return 1", target
		local link = Instance.new("ObjectValue")
		link.Name, link.Value, link.Parent = "Link", target, root
		local map = InstanceMap.new()
		for id, instance in { ROOT = root, PART = target, SCRIPT = source, LINK = link } do
			map:insert(id, instance)
		end
		local topology = LiveTopology.new(map)
		topology:seed({
			ROOT = { Name = "Root", ClassName = "Folder", Properties = {}, Children = { "PART", "LINK" } },
			PART = {
				Name = "Target",
				ClassName = "Part",
				Parent = "ROOT",
				Properties = {
					Anchored = { Bool = true },
					Transparency = { Float32 = 0 },
					Tags = { Tags = { "Managed" } },
					Attributes = { Attributes = { Owner = { String = "base" } } },
				},
				Children = { "SCRIPT" },
			},
			SCRIPT = {
				Name = "Child",
				ClassName = "ModuleScript",
				Parent = "PART",
				Properties = { Source = { String = "return 1" } },
				Children = {},
			},
			LINK = {
				Name = "Link",
				ClassName = "ObjectValue",
				Parent = "ROOT",
				Properties = { Value = { Ref = "PART" } },
				Children = {},
			},
		}, "ROOT")
		local originalReify = reify.reifyInstance
		local originalInsert = map.insert
		local ok, reason = pcall(callback, topology, map, root, target, source, link)
		reify.reifyInstance, map.insert = originalReify, originalInsert
		topology:stop()
		map:stop()
		root:Destroy()
		assert(ok, tostring(reason))
	end
	local function check(topology, protected, begin)
		topology:mark("PART")
		return topology:check(PatchSet.newEmpty(), true, protected or function()
			return {}
		end, begin or function()
			return function() end
		end)
	end

	it("retains copy subtree, attributes, tags, unknown descendants and outbound references", function()
		fixture(function(topology, map, root, target, _, link)
			local unknown = Instance.new("Folder")
			unknown.Name, unknown.Parent = "Unknown", target
			local replacement = target:Clone()
			replacement.Parent = root
			target:Destroy()
			local ok, reason = check(topology)
			assert(ok, tostring(reason))
			assert(
				map.fromIds.PART == replacement and map.fromIds.SCRIPT == replacement.Child,
				"subtree IDs not rebound"
			)
			assert(
				replacement.Unknown.Parent == replacement and replacement:GetAttribute("Owner") == "base",
				"copy data lost"
			)
			assert(replacement:HasTag("Managed") and link.Value == replacement, "tag/reference parity lost")
		end)
	end)

	it("refuses multiple copies without mutating either candidate", function()
		fixture(function(topology, map, root, target)
			local first, second = target:Clone(), target:Clone()
			first.Parent, second.Parent = root, root
			target:Destroy()
			local ok, reason = check(topology)
			assert(not ok and tostring(reason):find("multiple"), "ambiguous copies were accepted")
			assert(
				first.Parent == root and second.Parent == root and map.fromIds.PART == target,
				"refusal changed mappings/copies"
			)
		end)
	end)

	it("refuses changed properties, attributes, tags and nested Source", function()
		for _, mutate in
			{
				function(instance)
					instance.Transparency = 0.5
				end,
				function(instance)
					instance:SetAttribute("Owner", "different")
				end,
				function(instance)
					instance:AddTag("Different")
				end,
				function(instance)
					instance.Child.Source = "return 2"
				end,
			}
		do
			fixture(function(topology, map, root, target)
				local replacement = target:Clone()
				mutate(replacement)
				replacement.Parent = root
				target:Destroy()
				local ok = check(topology)
				assert(not ok and map.fromIds.PART == target and replacement.Parent == root, "changed copy was adopted")
			end)
		end
	end)

	it("protects dirty descendant Source after Destroy has erased ancestry", function()
		fixture(function(topology, map, root, target)
			local replacement = target:Clone()
			replacement.Parent = root
			target:Destroy()
			local ok, reason = check(topology, function()
				return { SCRIPT = true }
			end)
			assert(not ok and tostring(reason):find("Source"), "dirty descendant Source was not protected")
			assert(map.fromIds.PART == target and replacement.Parent == root, "protected refusal mutated objects")
		end)
	end)

	it("rechecks new Source protection before committing a prepared reconstruction", function()
		fixture(function(topology, map, root, target)
			target:Destroy()
			local reads = 0
			local ok, reason = check(topology, function()
				reads += 1
				return if reads == 1 then {} else { SCRIPT = true }
			end)
			assert(
				not ok and tostring(reason):find("new local Source"),
				"late Source edit did not revoke reconstruction"
			)
			assert(#root:GetChildren() == 1 and map.fromIds.PART == target, "revoked reconstruction escaped staging")
		end)
	end)

	it("discards staged instances when stopped during preparation", function()
		fixture(function(topology, map, root, target)
			target:Destroy()
			local original = reify.reifyInstance
			reify.reifyInstance = function(...)
				local result = original(...)
				topology:stop()
				return result
			end
			local ok = check(topology)
			assert(
				not ok and #root:GetChildren() == 1 and map.fromIds.PART == target,
				"stopped repair committed live state"
			)
		end)
	end)

	it("cleans incomplete staged subtrees before refusing uncreatable descendants", function()
		fixture(function(topology, map, root, target)
			target:Destroy()
			topology.records.SCRIPT.ClassName = "NotARealRobloxClass"
			local ok, reason = check(topology)
			assert(not ok and tostring(reason):find("faithfully"), "uncreatable subtree was accepted")
			assert(
				#root:GetChildren() == 1 and map.fromIds.PART == target,
				"incomplete staged tree mutated live mappings"
			)
		end)
	end)

	it("rolls back mappings and parenting if a commit cannot bind every ID", function()
		fixture(function(topology, map, root, target, source, link)
			target:Destroy()
			local previousReference = link.Value
			local insert, failed = map.insert, false
			map.insert = function(self, id, instance)
				if id == "SCRIPT" and instance ~= source and not failed then
					failed = true
					error("injected bind failure")
				end
				return insert(self, id, instance)
			end
			local ok, reason = check(topology)
			assert(not ok and tostring(reason):find("injected"), "mapping failure was hidden")
			assert(map.fromIds.PART == target and map.fromIds.SCRIPT == source, "failure left half-rebound mappings")
			assert(
				link.Value == previousReference and #root:GetChildren() == 1,
				("failure changed reference or left %d live children"):format(#root:GetChildren())
			)
		end)
	end)

	it("checks healthy property patches without traversing the whole instance map", function()
		fixture(function(topology, map)
			local traversals = 0
			setmetatable(map.fromIds, {
				__iter = function()
					traversals += 1
					return next, map.fromIds
				end,
			})
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "SCRIPT", changedProperties = { Source = { String = "return 2" } } })
			local ok, reason = topology:check(patch, true, function()
				error("healthy patch requested protection scan")
			end, function()
				error("healthy patch requested mutation")
			end)
			assert(ok and traversals == 0, tostring(reason or "healthy patch traversed all mappings"))
		end)
	end)
end
