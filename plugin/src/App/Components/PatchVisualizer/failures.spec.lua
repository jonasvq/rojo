return function()
	local Rojo = script:FindFirstAncestor("Rojo")
	local Plugin = Rojo.Plugin
	local Roact = require(Rojo.Packages.Roact)
	local PatchTree = require(Plugin.PatchTree)
	local PatchSet = require(Plugin.PatchSet)
	local Visualizer = require(script.Parent)
	local VirtualScroller = require(script.Parent.Parent.VirtualScroller)
	local failureRows = require(script.Parent.failureRows)

	local function fixture()
		local tree = PatchTree.build(PatchSet.newEmpty(), { fromIds = {}, fromInstances = {} })
		tree:addNode(nil, { id = "PARENT", name = "Container", className = "Folder" })
		for index = 1, 1000 do
			tree:addNode(
				"PARENT",
				{ id = "OK" .. index, name = string.format("Passed%04d", index), className = "Part" }
			)
		end
		tree:addNode("PARENT", {
			id = "FAILED",
			name = "Failed",
			className = "Part",
			isWarning = true,
			changeList = {
				{ "Property", "Old", "New" },
				{ "Color", "before", "after" },
				{ "Locked", false, true, { isWarning = true } },
			},
		})
		return tree
	end

	it("toggles the rendered list and offers all changes after failures disappear", function()
		local event = Instance.new("BindableEvent")
		local tree = fixture()
		local viewer = {
			props = { patchTree = tree, transparency = Roact.createBinding(0) },
			state = { failuresOnly = false },
			updateEvent = event,
			setState = function(self, update)
				self.state = update(self.state)
			end,
		}
		local function render()
			local themed = Visualizer.render(viewer)
			return themed.props.render({
				Font = { Main = Font.fromEnum(Enum.Font.SourceSans) },
				TextSize = { Medium = 14 },
				TextColor = Color3.new(1, 1, 1),
			}).props[Roact.Children]
		end
		local all = render()
		expect(all.VirtualScroller.props.count).to.equal(1002)
		expect(all.FailureFilter.props.text).to.equal("Failures only (1)")
		all.FailureFilter.props.onClick()
		local failed = render()
		expect(failed.FailureScroller.props.count).to.equal(2)
		expect(failed.VirtualScroller).to.equal(nil)
		local label = failed.FailureScroller.props.render(2)
		expect(label.props.name).to.equal("Failed")
		expect(#label.props.changeList).to.equal(2)
		failed.FailureFilter.props.onClick()
		expect(render().VirtualScroller.props.count).to.equal(1002)
		viewer.state.failuresOnly = true
		tree:getNode("FAILED").isWarning = nil
		local empty = render()
		expect(empty.FailureScroller.props.count).to.equal(0)
		expect(empty.CleanMerge.props.Visible).to.equal(true)
		expect(empty.CleanMerge.props.Text).to.equal("No failed changes in this view.")
		expect(empty.FailureFilter.props.text).to.equal("Show all changes")
		event:Destroy()
	end)

	it("shows one failed instance and its ancestor instead of a thousand successes", function()
		local event = Instance.new("BindableEvent")
		local viewer = {
			props = { patchTree = fixture(), transparency = Roact.createBinding(0) },
			state = { failuresOnly = true },
			updateEvent = event,
		}
		local themed = Visualizer.render(viewer)
		local frame = themed.props.render({
			Font = { Main = Font.fromEnum(Enum.Font.SourceSans) },
			TextSize = { Medium = 14 },
			TextColor = Color3.new(1, 1, 1),
		})
		local count
		for _, element in frame.props[Roact.Children] do
			if type(element) == "table" and element.component == VirtualScroller then
				count = element.props.count
			end
		end
		event:Destroy()
		expect(count).to.equal(2)
	end)

	it("keeps path context and failed properties without mutating the full view", function()
		local tree = fixture()
		local parent = tree:getNode("PARENT")
		parent.patchType = "Edit"
		parent.changeList = { { "Property", "Old", "New" }, { "Name", "old", "new" } }
		local all, failureCount = failureRows(tree, false)
		expect(#all).to.equal(1002)
		expect(failureCount).to.equal(1)
		local rows = failureRows(tree, true)
		expect(#rows).to.equal(2)
		expect(rows[1].node.id).to.equal("PARENT")
		expect(rows[1].node.changeList).to.equal(nil)
		expect(rows[1].hasChildren).to.equal(true)
		expect(rows[2].node.id).to.equal("FAILED")
		expect(rows[2].isFinalChild).to.equal(true)
		expect(#rows[2].node.changeList).to.equal(2)
		expect(rows[2].node.changeList[2][1]).to.equal("Locked")
		expect(#tree:getNode("FAILED").changeList).to.equal(3)
		expect(parent.patchType).to.equal("Edit")
		expect(#parent.changeList).to.equal(2)
	end)

	it("includes failed additions, removals, names and property resets", function()
		local tree = PatchTree.build(PatchSet.newEmpty(), { fromIds = {}, fromInstances = {} })
		tree:addNode(nil, { id = "PARENT", name = "Container", className = "Folder" })
		tree:addNode("PARENT", {
			id = "EDIT",
			name = "Edited",
			className = "Part",
			changeInfo = { edits = 3 },
			changeList = {
				{ "Property", "Old", "New" },
				{ "Color", "before", "after" },
				{ "Name", "old", "new" },
				{ "Archivable", false, true },
			},
		})
		tree:addNode("PARENT", {
			id = "ADD",
			name = "Added",
			className = "Folder",
			changeInfo = { edits = 1 },
			changeList = { { "Property", "Old", "New" }, { "Name", nil, "Added" } },
		})
		tree:addNode("PARENT", { id = "REMOVE", name = "Removed", className = "Folder" })
		local removed = Instance.new("Folder")
		local unapplied = PatchSet.newEmpty()
		unapplied.updated =
			{ { id = "EDIT", changedName = "new", changedProperties = {}, removedProperties = { "Archivable" } } }
		unapplied.added.ADD = {}
		unapplied.removed = { removed }
		tree = PatchTree.updateMetadata(
			tree,
			PatchSet.newEmpty(),
			{ fromIds = {}, fromInstances = { [removed] = "REMOVE" } },
			unapplied
		)
		removed:Destroy()
		local rows, count = failureRows(tree, true)
		expect(count).to.equal(3)
		expect(#rows).to.equal(4)
		local edited
		for _, row in rows do
			if row.node.id == "EDIT" then
				edited = row.node
			end
		end
		expect(#edited.changeList).to.equal(3)
		expect(edited.changeList[2][1]).to.equal("Name")
		expect(edited.changeList[3][1]).to.equal("Archivable")
	end)

	it("keeps failed ancestors once and hides successful descendants", function()
		local tree = fixture()
		tree:addNode("FAILED", { id = "NESTED", name = "Nested failure", className = "Folder", isWarning = true })
		tree:addNode("FAILED", { id = "OK_CHILD", name = "Passed child", className = "Folder" })
		local rows, count = failureRows(tree, true)
		expect(count).to.equal(2)
		expect(#rows).to.equal(3)
		expect(rows[1].depth).to.equal(1)
		expect(rows[2].depth).to.equal(2)
		expect(rows[3].depth).to.equal(3)
		expect(rows[3].node.id).to.equal("NESTED")
	end)

	it("updates when only failure metadata changes", function()
		local props = { patch = PatchSet.newEmpty(), unappliedPatch = PatchSet.newEmpty() }
		local nextProps = table.clone(props)
		nextProps.unappliedPatch = PatchSet.newEmpty()
		expect(
			Visualizer.shouldUpdate(
				{ props = props, state = { failuresOnly = false } },
				nextProps,
				{ failuresOnly = false }
			)
		).to.equal(true)
	end)

	it("has an empty failure view without claiming there are no successful changes", function()
		local tree = fixture()
		tree:getNode("FAILED").isWarning = nil
		local rows, count = failureRows(tree, true)
		expect(#rows).to.equal(0)
		expect(count).to.equal(0)
		local all = failureRows(tree, false)
		expect(#all).to.equal(1002)
		local empty, emptyCount = failureRows(nil, true)
		expect(#empty).to.equal(0)
		expect(emptyCount).to.equal(0)
	end)

	it("rerenders when the failure filter changes", function()
		local props = { patchTree = fixture() }
		expect(
			Visualizer.shouldUpdate({ props = props, state = { failuresOnly = false } }, props, { failuresOnly = true })
		).to.equal(true)
	end)
end
