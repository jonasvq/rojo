return function()
	local applyPatch = require(script.Parent.applyPatch)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local function fixture()
		local root = Instance.new("Folder")
		local original = Instance.new("Part")
		original.Name, original.Anchored, original.Parent = "SavedPart", true, root
		local map = InstanceMap.new()
		map:insert("ROOT", root)
		local patch = {
			removed = {},
			updated = {},
			added = {
				PART = {
					Id = "PART",
					Parent = "ROOT",
					Name = "SavedPart",
					ClassName = "Part",
					Properties = { Anchored = { Bool = true } },
					Children = {},
				},
			},
		}
		return root, original, map, patch
	end
	describe("live saved-instance adoption", function()
		local function check(callback)
			local root, original, map, patch = fixture()
			local ok, reason = pcall(callback, root, original, map, patch)
			map:stop()
			root:Destroy()
			assert(ok, tostring(reason))
		end
		it("retains legacy creation when adoption is disabled", function()
			check(function(root, original, map, patch)
				applyPatch(map, patch)
				assert(
					#root:GetChildren() == 2 and map.fromIds.PART ~= original,
					"disabled adoption changed legacy creation"
				)
			end)
		end)
		it("creates entirely absent roots beside adopted roots", function()
			check(function(root, original, map, patch)
				patch.added.NEW = {
					Id = "NEW",
					Parent = "ROOT",
					Name = "NewFolder",
					ClassName = "Folder",
					Properties = {},
					Children = {},
				}
				local _, conflict = applyPatch(map, patch, true)
				assert(
					not conflict and map.fromIds.PART == original and root.NewFolder == map.fromIds.NEW,
					"mixed creation lost identity"
				)
				assert(#root:GetChildren() == 2, "mixed batch created unexpected siblings")
			end)
		end)
		it("preserves known serialization-only fields without asserting binary equivalence", function()
			check(function(_, original, map, patch)
				patch.added.PART.Properties.SerializedOverrides = { BinaryString = "opaque file value" }
				local _, conflict = applyPatch(map, patch, true)
				assert(
					not conflict and map.fromIds.PART == original,
					"opaque field unexpectedly blocked live-equivalent adoption"
				)
			end)
		end)
		it("indexes wide sibling groups only once", function()
			local adopt = require(script.Parent.adoptAdded)
			local children, added, reads = {}, {}, 0
			local parent = {
				Parent = nil,
				GetChildren = function()
					reads += 1
					return children
				end,
			}
			local map = { fromIds = { ROOT = parent }, fromInstances = {} }
			function map:insert(id, instance)
				self.fromIds[id], self.fromInstances[instance] = instance, id
			end
			for index = 1, 2000 do
				local id = "FOLDER" .. index
				local instance = {
					Name = id,
					ClassName = "Folder",
					Archivable = true,
					Parent = parent,
					GetChildren = function()
						return {}
					end,
				}
				table.insert(children, instance)
				added[id] =
					{ Id = id, Parent = "ROOT", Name = id, ClassName = "Folder", Properties = {}, Children = {} }
			end
			local ok, reason = adopt(map, { removed = {}, updated = {}, added = added })
			assert(ok, tostring(reason))
			assert(reads == 1 and map.fromIds.FOLDER2000 == children[2000], "wide adoption repeatedly scanned siblings")
		end)
		it("rejects ambiguous, wrong-class, already-owned and unreadable candidates", function()
			for _, kind in { "ambiguous", "class", "owned", "unreadable" } do
				check(function(root, original, map, patch)
					if kind == "ambiguous" then
						original:Clone().Parent = root
					elseif kind == "class" then
						patch.added.PART.ClassName = "Folder"
					elseif kind == "owned" then
						map:insert("OLD", original)
					else
						patch.added.PART.Properties.NotAProperty = { Bool = true }
					end
					local _, conflict = applyPatch(map, patch, true)
					assert(conflict and map.fromIds.PART == nil and original.Parent == root, kind)
					if kind == "owned" then
						assert(map.fromIds.OLD == original, "refusal replaced an existing managed binding")
					end
				end)
			end
		end)
		it("validates nested cycles and external references without changing unknown descendants", function()
			check(function(root, original, map, patch)
				local unknown = Instance.new("Folder")
				unknown.Name, unknown.Parent = "Unmentioned", original
				local a, b = Instance.new("ObjectValue"), Instance.new("ObjectValue")
				a.Name, b.Name, a.Parent, b.Parent, a.Value, b.Value = "A", "B", original, root, b, a
				patch.added.PART.Children = { "A" }
				patch.added.A = {
					Id = "A",
					Parent = "PART",
					Name = "A",
					ClassName = "ObjectValue",
					Properties = { Value = { Ref = "B" } },
					Children = {},
				}
				patch.added.B = {
					Id = "B",
					Parent = "ROOT",
					Name = "B",
					ClassName = "ObjectValue",
					Properties = { Value = { Ref = "A" } },
					Children = {},
				}
				local _, conflict = applyPatch(map, patch, true)
				assert(not conflict, tostring(conflict))
				assert(
					map.fromIds.PART == original and map.fromIds.A == a and map.fromIds.B == b,
					"nested adoption lost identity"
				)
				assert(
					unknown.Parent == original and not map.fromInstances[unknown] and a.Value == b and b.Value == a,
					"adoption changed references or unknown descendants"
				)
			end)
		end)
		it("rejects missing subtree children and unresolved references without partial bindings", function()
			for _, kind in { "missing", "ref", "removed" } do
				check(function(root, original, map, patch)
					if kind == "missing" then
						patch.added.PART.Children = { "MISSING" }
						patch.added.MISSING = {
							Id = "MISSING",
							Parent = "PART",
							Name = "Missing",
							ClassName = "Folder",
							Properties = {},
							Children = {},
						}
					elseif kind == "ref" then
						patch.added.PART.Properties.Parent = { Ref = "MISSING" }
					else
						table.insert(patch.removed, "ROOT")
					end
					local _, conflict = applyPatch(map, patch, true)
					assert(
						conflict and not map.fromIds.PART and original.Parent == root and #root:GetChildren() == 1,
						kind
					)
				end)
			end
		end)
		it("rolls back tentative mappings when insertion fails", function()
			check(function(_, original, map, patch)
				local insert = map.insert
				map.insert = function(self, id, instance)
					insert(self, id, instance)
					error("injected mapping failure")
				end
				local _, conflict = applyPatch(map, patch, true)
				assert(
					conflict and not map.fromIds.PART and not map.fromInstances[original],
					"failed insertion left a partial binding"
				)
				assert(map.fromIds.ROOT, "rollback lost the original root binding")
			end)
		end)
		it("bounds deep forests before binding any node", function()
			check(function(_, original, map, patch)
				local parent, parentId = original, "PART"
				for index = 1, 129 do
					local id = "DEEP" .. index
					local child = Instance.new("Folder")
					child.Name, child.Parent = id, parent
					patch.added[parentId].Children = { id }
					patch.added[id] =
						{ Id = id, Parent = parentId, Name = id, ClassName = "Folder", Properties = {}, Children = {} }
					parent, parentId = child, id
				end
				local _, conflict = applyPatch(map, patch, true)
				assert(
					conflict and not map.fromIds.PART and map:size() == 1,
					"deep forest bound committed partial mappings"
				)
			end)
		end)
		it("adopts equivalent untracked instances only with explicit opt-in", function()
			local root, original, map, patch = fixture()
			local _, conflict = applyPatch(map, patch, true)
			local adopted = map.fromIds.PART == original and #root:GetChildren() == 1
			map:stop()
			root:Destroy()
			assert(not conflict and adopted, "opted-in saved instance was duplicated instead of adopted")
		end)
		it("refuses a differing property before removing or updating anything", function()
			local root, original, map, patch = fixture()
			original.Anchored = false
			table.insert(patch.updated, { id = "ROOT", changedName = "Wrong", changedProperties = {} })
			local _, conflict = applyPatch(map, patch, true)
			local untouched = root.Name == "Folder" and #root:GetChildren() == 1 and not original.Anchored
			local unmapped = map.fromIds.PART == nil
			map:stop()
			root:Destroy()
			assert(conflict and untouched and unmapped, "adoption conflict applied part of the batch")
		end)
	end)
end
