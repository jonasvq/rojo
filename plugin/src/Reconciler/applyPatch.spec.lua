return function()
	local applyPatch = require(script.Parent.applyPatch)

	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)

	local container = Instance.new("Folder")

	local tempContainer = Instance.new("Folder")
	local function wasRemoved(instance)
		-- If an instance was destroyed, its parent property is locked.
		-- If an instance was removed, its parent property is nil.
		-- We need to ensure we only remove, so that ChangeHistoryService can still Undo.

		local isParentUnlocked = pcall(function()
			local oldParent = instance.Parent
			instance.Parent = tempContainer
			instance.Parent = oldParent
		end)

		return instance.Parent == nil and isParentUnlocked
	end

	beforeEach(function()
		container:ClearAllChildren()
	end)

	afterAll(function()
		container:Destroy()
		tempContainer:Destroy()
	end)

	it("should return an empty patch if given an empty patch", function()
		local patch = applyPatch(InstanceMap.new(), PatchSet.newEmpty())
		assert(PatchSet.isEmpty(patch), "expected remaining patch to be empty")
	end)

	it("retains an entire missing-parent addition for retry while applying unrelated valid siblings", function()
		local map = InstanceMap.new()
		map:insert("ROOT", container)
		local patch = PatchSet.newEmpty()
		patch.added = {
			CHILD = {
				Id = "CHILD",
				Parent = "PARENT",
				Name = "Child",
				ClassName = "Folder",
				Properties = {},
				Children = { "GRANDCHILD" },
			},
			GRANDCHILD = {
				Id = "GRANDCHILD",
				Parent = "CHILD",
				Name = "Grandchild",
				ClassName = "Part",
				Properties = {},
				Children = {},
			},
			HEALTHY = {
				Id = "HEALTHY",
				Parent = "ROOT",
				Name = "Healthy",
				ClassName = "Folder",
				Properties = {},
				Children = {},
			},
		}
		local ok, err = pcall(function()
			local unapplied = applyPatch(map, patch)
			expect(unapplied.added.CHILD).to.equal(patch.added.CHILD)
			expect(unapplied.added.GRANDCHILD).to.equal(patch.added.GRANDCHILD)
			expect(unapplied.added.HEALTHY).to.equal(nil)
			expect(map.fromIds.HEALTHY.Parent).to.equal(container)
			expect(map.fromIds.CHILD).to.equal(nil)
			expect(map.fromIds.GRANDCHILD).to.equal(nil)
			local parentPatch = PatchSet.newEmpty()
			parentPatch.added.PARENT = {
				Id = "PARENT",
				Parent = "ROOT",
				Name = "Parent",
				ClassName = "Folder",
				Properties = {},
				Children = {},
			}
			expect(PatchSet.isEmpty(applyPatch(map, parentPatch))).to.equal(true)
			expect(PatchSet.isEmpty(applyPatch(map, unapplied))).to.equal(true)
			expect(map.fromIds.CHILD.Parent).to.equal(map.fromIds.PARENT)
			expect(map.fromIds.GRANDCHILD.Parent).to.equal(map.fromIds.CHILD)
		end)
		map:stop()
		assert(ok, tostring(err))
	end)

	it("reports later child additions as unapplied after the parent class was rejected", function()
		local map = InstanceMap.new()
		map:insert("ROOT", container)
		local parent = PatchSet.newEmpty()
		parent.added.PARENT = {
			Id = "PARENT",
			Parent = "ROOT",
			Name = "RejectedParent",
			ClassName = "DefinitelyMissingClassForTest",
			Properties = {},
			Children = {},
		}
		local ok, err = pcall(function()
			expect(applyPatch(map, parent).added.PARENT).to.equal(parent.added.PARENT)
			local child = PatchSet.newEmpty()
			child.added.CHILD =
				{ Id = "CHILD", Parent = "PARENT", Name = "Child", ClassName = "Part", Properties = {}, Children = {} }
			expect(applyPatch(map, child).added.CHILD).to.equal(child.added.CHILD)
			expect(map.fromIds.CHILD).to.equal(nil)
			expect(#container:GetChildren()).to.equal(0)
		end)
		map:stop()
		assert(ok, tostring(err))
	end)

	it("retains strict adoption preflight refusal when an external parent is missing", function()
		local map = InstanceMap.new()
		map:insert("ROOT", container)
		local patch = PatchSet.newEmpty()
		patch.added.CHILD =
			{ Id = "CHILD", Parent = "MISSING", Name = "Child", ClassName = "Part", Properties = {}, Children = {} }
		local unapplied, reason = applyPatch(map, patch, true)
		expect(unapplied.added.CHILD).to.equal(patch.added.CHILD)
		assert(type(reason) == "string" and reason:find("Missing adoption parent", 1, true), tostring(reason))
		expect(#container:GetChildren()).to.equal(0)
		map:stop()
	end)

	it("keeps missing-parent additions visible to ServeSession postcommit without disconnecting", function()
		local ServeSession = require(script.Parent.Parent.ServeSession)
		local Settings = require(script.Parent.Parent.Settings)
		local previousFallback = Settings:get("enableSyncFallback")
		Settings:set("enableSyncFallback", false)
		local session = ServeSession.new({ apiContext = { disconnect = function() end }, twoWaySync = false })
		session.__status = ServeSession.Status.Connected
		session.__instanceMap:insert("ROOT", container)
		local observed
		session:hookPostcommit(function(_, _, unapplied)
			observed = unapplied
		end)
		local patch = PatchSet.newEmpty()
		patch.added.CHILD =
			{ Id = "CHILD", Parent = "MISSING", Name = "Child", ClassName = "Part", Properties = {}, Children = {} }
		local ok, err = pcall(function()
			session:__applyPatch(patch)
			task.wait()
			expect(session.__status).to.equal(ServeSession.Status.Connected)
			expect(observed.added.CHILD).to.equal(patch.added.CHILD)
			expect(session.__instanceMap.fromIds.CHILD).to.equal(nil)
		end)
		session:stop()
		Settings:set("enableSyncFallback", previousFallback)
		assert(ok, tostring(err))
	end)

	it("acknowledges live sync policy metadata without replacing mapped instances", function()
		local instance = Instance.new("ModuleScript")
		instance.Source = "return 1"
		instance.Parent = container
		local child = Instance.new("Folder")
		child.Parent = instance
		local map = InstanceMap.new()
		map:insert("MODULE", instance)
		for _, ignore in { true, false } do
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "MODULE",
				changedMetadata = { ignoreUnknownInstances = ignore },
				changedProperties = { Source = { String = "return 2" } },
			})
			local unapplied = applyPatch(map, patch)
			assert(PatchSet.isEmpty(unapplied), "metadata unnecessarily requested serialization fallback")
			assert(map.fromIds.MODULE == instance and instance.Parent == container, "metadata replaced identity")
			assert(child.Parent == instance and instance.Source == "return 2", "metadata lost children or source")
		end
		map:stop()
	end)

	it("should remove instances listed for remove", function()
		local root = Instance.new("Folder")
		root.Name = "ROOT"
		root.Parent = container

		local child = Instance.new("Folder")
		child.Name = "Child"
		child.Parent = root

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)
		instanceMap:insert("CHILD", child)

		local patch = PatchSet.newEmpty()
		table.insert(patch.removed, child)

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")

		assert(not wasRemoved(root), "expected root to be left alone")
		assert(wasRemoved(child), "expected child to be removed")

		instanceMap:stop()
	end)

	it("should remove IDs listed for remove", function()
		local root = Instance.new("Folder")
		root.Name = "ROOT"
		root.Parent = container

		local child = Instance.new("Folder")
		child.Name = "Child"
		child.Parent = root

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)
		instanceMap:insert("CHILD", child)

		local patch = PatchSet.newEmpty()
		table.insert(patch.removed, "CHILD")

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")
		expect(instanceMap:size()).to.equal(1)

		assert(not wasRemoved(root), "expected root to be left alone")
		assert(wasRemoved(child), "expected child to be removed")

		instanceMap:stop()
	end)

	it("should add instances to the DOM", function()
		-- Many of the details of this functionality are instead covered by
		-- tests on reify, not here.

		local root = Instance.new("Folder")
		root.Name = "ROOT"
		root.Parent = container

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)

		local patch = PatchSet.newEmpty()
		patch.added["CHILD"] = {
			Id = "CHILD",
			ClassName = "Model",
			Name = "Child",
			Parent = "ROOT",
			Children = { "GRANDCHILD" },
			Properties = {},
		}

		patch.added["GRANDCHILD"] = {
			Id = "GRANDCHILD",
			ClassName = "Part",
			Name = "Grandchild",
			Parent = "CHILD",
			Children = {},
			Properties = {},
		}

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")
		expect(instanceMap:size()).to.equal(3)

		local child = root:FindFirstChild("Child")
		expect(child).to.be.ok()
		expect(child.ClassName).to.equal("Model")
		expect(child).to.equal(instanceMap.fromIds["CHILD"])

		local grandchild = child:FindFirstChild("Grandchild")
		expect(grandchild).to.be.ok()
		expect(grandchild.ClassName).to.equal("Part")
		expect(grandchild).to.equal(instanceMap.fromIds["GRANDCHILD"])
	end)

	it("should return unapplied additions when instances cannot be created", function()
		local root = Instance.new("Folder")
		root.Name = "ROOT"
		root.Parent = container

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)

		local patch = PatchSet.newEmpty()
		patch.added["OOPSIE"] = {
			Id = "OOPSIE",
			-- Hopefully Roblox never makes an instance with this ClassName.
			ClassName = "UH OH",
			Name = "FUBAR",
			Parent = "ROOT",
			Children = {},
			Properties = {},
		}

		local unapplied = applyPatch(instanceMap, patch)
		expect(unapplied.added["OOPSIE"]).to.equal(patch.added["OOPSIE"])
		expect(instanceMap:size()).to.equal(1)
		expect(#root:GetChildren()).to.equal(0)
	end)

	it("should apply property changes to instances", function()
		local value = Instance.new("StringValue")
		value.Value = "HELLO"

		local instanceMap = InstanceMap.new()
		instanceMap:insert("VALUE", value)

		local patch = PatchSet.newEmpty()
		table.insert(patch.updated, {
			id = "VALUE",
			changedProperties = {
				Value = {
					String = "WORLD",
				},
			},
		})

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")
		expect(value.Value).to.equal("WORLD")
	end)

	it("preserves untracked children when switching between legacy and RunContext scripts", function()
		local original = Instance.new("LocalScript")
		original.Name = "Client"
		original.Parent = container
		local child = Instance.new("Folder")
		child.Name = "StudioOnly"
		child.Parent = original
		local value = Instance.new("StringValue")
		value.Value = "keep me"
		value.Parent = child
		local map = InstanceMap.new()
		map:insert("CLIENT", original)
		for _, class in { "Script", "LocalScript" } do
			local patch = PatchSet.newEmpty()
			table.insert(
				patch.updated,
				{ id = "CLIENT", changedClassName = class, changedProperties = { Source = { String = "return 1" } } }
			)
			local unapplied = applyPatch(map, patch)
			assert(PatchSet.isEmpty(unapplied), "Class change did not apply")
			local current = map.fromIds.CLIENT
			assert(current.ClassName == class, "Wrong script class")
			assert(
				child.Parent == current and value.Parent == child and value.Value == "keep me",
				"Untracked descendants were lost"
			)
			assert(map.fromInstances[child] == nil, "Unknown child was adopted")
		end
	end)

	it("should recreate instances when changedClassName is set, preserving children", function()
		local root = Instance.new("Folder")
		root.Name = "Initial Root Name"
		root.Parent = container

		local child = Instance.new("Folder")
		child.Name = "Child"
		child.Parent = root

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)
		instanceMap:insert("CHILD", child)

		local patch = PatchSet.newEmpty()
		table.insert(patch.updated, {
			id = "ROOT",
			changedName = "Updated Root Name",
			changedClassName = "StringValue",
			changedProperties = {
				Value = {
					String = "I am Root",
				},
			},
		})

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")

		local newRoot = instanceMap.fromIds["ROOT"]
		assert(newRoot ~= root, "expected instance to be recreated")
		expect(newRoot.ClassName).to.equal("StringValue")
		expect(newRoot.Name).to.equal("Updated Root Name")
		expect(newRoot.Value).to.equal("I am Root")

		local newChild = newRoot:FindFirstChild("Child")
		assert(newChild ~= nil, "expected child to be present")
		assert(newChild == child, "expected child to be preserved")
	end)
end
