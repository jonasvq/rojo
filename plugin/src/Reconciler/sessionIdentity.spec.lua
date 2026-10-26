return function()
	local Reconciler = require(script.Parent)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)

	local function tree(children, nodes)
		nodes = nodes or {}
		nodes.ROOT = {
			ClassName = "Folder",
			Name = "Root",
			Properties = {},
			Children = children,
			Metadata = { ignoreUnknownInstances = true },
		}
		return nodes
	end

	local function remembered(root, nodes, sessionId)
		local map = InstanceMap.new()
		local reconciler = Reconciler.new(map)
		reconciler:hydrate(nodes, "ROOT", root, sessionId)
		reconciler:rememberSession(sessionId, root)
		map:stop()
	end

	it("removes previously managed children deleted offline while preserving unknown children", function()
		local root = Instance.new("Folder")
		local owned = Instance.new("Folder")
		owned.Name = "Owned"
		owned.Parent = root
		local foreign = Instance.new("Folder")
		foreign.Name = "Foreign"
		foreign.Parent = root
		remembered(
			root,
			tree({ "OWNED" }, {
				OWNED = { ClassName = "Folder", Name = "Owned", Properties = {}, Children = {} },
			}),
			"offline-deletion"
		)
		local map = InstanceMap.new()
		local reconciler = Reconciler.new(map)
		local nodes = tree({})
		reconciler:hydrate(nodes, "ROOT", root, "offline-deletion")
		local ok, patch = reconciler:diff(nodes, "ROOT")
		assert(ok, tostring(patch))
		reconciler:applyPatch(patch)
		local removed = owned.Parent == nil
		local preserved = foreign.Parent == root
		map:stop()
		root:Destroy()
		owned:Destroy()
		assert(removed, "A managed child deleted offline remained in Studio")
		assert(preserved, "An unknown child was removed")
	end)

	it("reserves remembered siblings before matching by name and class", function()
		local root = Instance.new("Folder")
		local first = Instance.new("LocalScript")
		first.Name = "Same"
		first.Parent = root
		local second = Instance.new("Folder")
		second.Name = "Same"
		second.Parent = root
		local nodes = tree({ "FIRST", "SECOND" }, {
			FIRST = { ClassName = "LocalScript", Name = "Same", Properties = {}, Children = {} },
			SECOND = { ClassName = "Folder", Name = "Same", Properties = {}, Children = {} },
		})
		remembered(root, nodes, "reserved-siblings")
		nodes.FIRST.ClassName = "Folder"
		local map = InstanceMap.new()
		Reconciler.new(map):hydrate(nodes, "ROOT", root, "reserved-siblings")
		local firstMatch, secondMatch = map.fromIds.FIRST, map.fromIds.SECOND
		map:stop()
		root:Destroy()
		expect(firstMatch).to.equal(first)
		expect(secondMatch).to.equal(second)
	end)

	it("does not reuse an offline-deleted instance for a newly created ID of the same name", function()
		local root = Instance.new("Folder")
		local old = Instance.new("Folder")
		old.Name = "Same"
		old.Parent = root
		remembered(
			root,
			tree({ "OLD" }, {
				OLD = { ClassName = "Folder", Name = "Same", Properties = {}, Children = {} },
			}),
			"deleted-and-recreated"
		)
		local nodes = tree({ "NEW" }, {
			NEW = { Id = "NEW", Parent = "ROOT", ClassName = "Folder", Name = "Same", Properties = {}, Children = {} },
		})
		local map = InstanceMap.new()
		local reconciler = Reconciler.new(map)
		reconciler:hydrate(nodes, "ROOT", root, "deleted-and-recreated")
		local matched = map.fromIds.NEW
		local ok, patch = reconciler:diff(nodes, "ROOT")
		assert(ok, tostring(patch))
		reconciler:applyPatch(patch)
		local replacement = map.fromIds.NEW
		local oldParent = old.Parent
		map:stop()
		root:Destroy()
		old:Destroy()
		expect(matched).to.equal(nil)
		assert(replacement ~= nil and replacement ~= old, "A new ID must create a new instance")
		expect(oldParent).to.equal(nil)
	end)

	it("does not reuse identity with a different root instance or root ID", function()
		for _, changeRootId in { false, true } do
			local root = Instance.new("Folder")
			local otherRoot = Instance.new("Folder")
			local old = Instance.new("LocalScript")
			old.Name = "Client"
			old.Parent = root
			local nodes = tree({ "CLIENT" }, {
				CLIENT = { ClassName = "LocalScript", Name = "Client", Properties = {}, Children = {} },
			})
			remembered(root, nodes, "root-isolation")
			nodes.CLIENT.ClassName = "Folder"
			local rootId = "ROOT"
			local currentRoot = otherRoot
			if changeRootId then
				rootId = "OTHER_ROOT"
				nodes.OTHER_ROOT = nodes.ROOT
				nodes.ROOT = nil
				currentRoot = root
			else
				old.Parent = otherRoot
			end
			local map = InstanceMap.new()
			Reconciler.new(map):hydrate(nodes, rootId, currentRoot, "root-isolation")
			local match = map.fromIds.CLIENT
			map:stop()
			root:Destroy()
			otherRoot:Destroy()
			expect(match).to.equal(nil)
		end
	end)

	it("does not transfer remembered ownership to a different server", function()
		local root = Instance.new("Folder")
		local old = Instance.new("LocalScript")
		old.Name = "Client"
		old.Parent = root
		local nodes = tree({ "CLIENT" }, {
			CLIENT = { ClassName = "LocalScript", Name = "Client", Properties = {}, Children = {} },
		})
		remembered(root, nodes, "old-server")
		nodes.CLIENT.ClassName = "Folder"
		local map = InstanceMap.new()
		Reconciler.new(map):hydrate(nodes, "ROOT", root, "new-server")
		local match = map.fromIds.CLIENT
		map:stop()
		root:Destroy()
		expect(match).to.equal(nil)
	end)

	it("does not reclaim a remembered instance that was moved outside its parent", function()
		local root = Instance.new("Folder")
		local outside = Instance.new("Folder")
		local old = Instance.new("LocalScript")
		old.Name = "Client"
		old.Parent = root
		local nodes = tree({ "CLIENT" }, {
			CLIENT = { ClassName = "LocalScript", Name = "Client", Properties = {}, Children = {} },
		})
		remembered(root, nodes, "moved-child")
		old.Parent = outside
		nodes.CLIENT.ClassName = "Folder"
		local map = InstanceMap.new()
		Reconciler.new(map):hydrate(nodes, "ROOT", root, "moved-child")
		local match = map.fromIds.CLIENT
		map:stop()
		root:Destroy()
		outside:Destroy()
		expect(match).to.equal(nil)
	end)
end
