return function()
	local resetProperty = require(script.Parent.resetProperty)
	local applyPatch = require(script.Parent.applyPatch)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local PatchTree = require(script.Parent.Parent.PatchTree)

	it("clears modification state for the UI properties in the original report", function()
		for _, case in
			{
				{ "TextLabel", "Position", UDim2.fromOffset(0, 2) },
				{ "Frame", "Size", UDim2.fromOffset(320, 200) },
				{ "TextButton", "AutoButtonColor", false },
				{ "StyleRule", "Priority", 20 },
			}
		do
			local instance = Instance.new(case[1])
			local property = case[2]
			local default = instance[property]
			instance[property] = case[3]
			assert(instance:IsPropertyModified(property))
			local ok, err = resetProperty(instance, property)
			assert(ok, tostring(err))
			expect(instance[property]).to.equal(default)
			expect(instance:IsPropertyModified(property)).to.equal(false)
			instance:Destroy()
		end
	end)

	it("resets a property without replacing the instance or changing unrelated values", function()
		local part = Instance.new("Part")
		part.Transparency = 0.5
		part.Anchored = true
		local map = InstanceMap.new()
		map:insert("PART", part)
		local patch = {
			removed = {},
			added = {},
			updated = {
				{
					id = "PART",
					changedProperties = {},
					removedProperties = { "Transparency" },
				},
			},
		}
		expect(PatchSet.countChanges(patch)).to.equal(1)
		local tree = PatchTree.build(patch, map, { "Property", "Before", "After" })
		expect(tree:getNode("PART").changeList[2][3]).to.equal("[Reset to default]")
		local remaining = applyPatch(map, patch)
		assert(PatchSet.isEmpty(remaining))
		expect(map.fromIds.PART).to.equal(part)
		expect(part.Transparency).to.equal(0)
		expect(part:IsPropertyModified("Transparency")).to.equal(false)
		expect(part.Anchored).to.equal(true)
		map:stop()
		part:Destroy()
	end)

	it("resets reference properties and synthetic attributes and tags", function()
		local value = Instance.new("ObjectValue")
		local target = Instance.new("Folder")
		value.Value = target
		value:SetAttribute("Example", 12)
		value:AddTag("ExampleTag")
		for _, property in { "Value", "Attributes", "Tags" } do
			local ok, err = resetProperty(value, property)
			assert(ok, tostring(err))
		end
		expect(value.Value).to.equal(nil)
		expect(value:IsPropertyModified("Value")).to.equal(false)
		expect(next(value:GetAttributes())).to.equal(nil)
		expect(#value:GetTags()).to.equal(0)
		value:Destroy()
		target:Destroy()
	end)

	it("uses canonical aliases and descriptor defaults for synthetic values", function()
		local part = Instance.new("Part")
		part.Color = Color3.new(1, 0, 0)
		local ok, err = resetProperty(part, "Color3uint8")
		assert(ok, tostring(err))
		expect(part:IsPropertyModified("Color")).to.equal(false)
		part:Destroy()
		local model = Instance.new("Model")
		model:ScaleTo(2)
		ok, err = resetProperty(model, "Scale")
		assert(ok, tostring(err))
		expect(model:GetScale()).to.equal(1)
		model:Destroy()
	end)

	it("retains failed resets in the unapplied patch and visualizer", function()
		local part = Instance.new("Part")
		local map = InstanceMap.new()
		map:insert("PART", part)
		local patch = {
			removed = {},
			added = {},
			updated = {
				{
					id = "PART",
					changedProperties = {},
					removedProperties = { "NotAProperty", "ClassName" },
				},
			},
		}
		local tree = PatchTree.build(patch, map, { "Property", "Before", "After" })
		local remaining = applyPatch(map, patch)
		expect(#remaining.updated).to.equal(1)
		expect(#remaining.updated[1].removedProperties).to.equal(2)
		PatchTree.updateMetadata(tree, patch, map, remaining)
		expect(tree:getNode("PART").isWarning).to.equal(true)
		map:stop()
		part:Destroy()
	end)

	it("keeps fresh class replacement defaults when old properties are removed", function()
		local root = Instance.new("StringValue")
		root.Value = "old"
		local map = InstanceMap.new()
		map:insert("ROOT", root)
		local remaining = applyPatch(map, {
			removed = {},
			added = {},
			updated = {
				{
					id = "ROOT",
					changedClassName = "NumberValue",
					changedProperties = {},
					removedProperties = { "Value" },
				},
			},
		})
		assert(PatchSet.isEmpty(remaining))
		expect(map.fromIds.ROOT.Value).to.equal(0)
		map.fromIds.ROOT:Destroy()
		root:Destroy()
		map:stop()
	end)
end
