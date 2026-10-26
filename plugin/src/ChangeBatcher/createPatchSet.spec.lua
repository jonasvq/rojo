return function()
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)

	local createPatchSet = require(script.Parent.createPatchSet)

	it("should return a patch", function()
		local patch = createPatchSet(InstanceMap.new(), {})

		assert(PatchSet.validate(patch))
	end)

	it("should contain updates for every instance with property changes", function()
		local instanceMap = InstanceMap.new()

		local part1 = Instance.new("Part")
		instanceMap:insert("PART_1", part1)

		local part2 = Instance.new("Part")
		instanceMap:insert("PART_2", part2)

		local changes = {
			[part1] = {
				Position = true,
				Size = true,
				Color = true,
			},
			[part2] = {
				CFrame = true,
				Velocity = true,
				Transparency = true,
			},
		}

		local patch = createPatchSet(instanceMap, changes)

		expect(#patch.updated).to.equal(2)
	end)

	it("should not contain any updates for removed instances", function()
		local instanceMap = InstanceMap.new()

		local part1 = Instance.new("Part")
		instanceMap:insert("PART_1", part1)

		local changes = {
			[part1] = {
				Parent = true,
				Position = true,
				Size = true,
			},
		}

		local patch = createPatchSet(instanceMap, changes)

		expect(#patch.removed).to.equal(1)
		expect(#patch.updated).to.equal(0)
	end)

	it("should remove instances from the property change table", function()
		local instanceMap = InstanceMap.new()

		local part1 = Instance.new("Part")
		instanceMap:insert("PART_1", part1)

		local changes = {
			[part1] = {},
		}

		createPatchSet(instanceMap, changes)

		expect(next(changes)).to.equal(nil)
	end)
	it("encodes tracked references and nil references without throwing", function()
		local instanceMap = InstanceMap.new()
		local model = Instance.new("Model")
		local part = Instance.new("Part")
		part.Parent = model
		model.PrimaryPart = part
		local pointer = Instance.new("ObjectValue")
		local modelId = "11111111111111111111111111111111"
		local partId = "22222222222222222222222222222222"
		local pointerId = "33333333333333333333333333333333"
		instanceMap:insert(modelId, model)
		instanceMap:insert(partId, part)
		instanceMap:insert(pointerId, pointer)
		local changes = { [model] = { PrimaryPart = true }, [pointer] = { Value = true } }
		local ok, patch = pcall(createPatchSet, instanceMap, changes)
		instanceMap:stop()
		model:Destroy()
		pointer:Destroy()
		assert(ok, tostring(patch))
		local updates = {}
		for _, update in patch.updated do
			updates[update.id] = update
		end
		expect(updates[modelId].changedProperties.PrimaryPart.Ref).to.equal(partId)
		expect(updates[pointerId].changedProperties.Value.Ref).to.equal("00000000000000000000000000000000")
		expect(next(changes)).to.equal(nil)
	end)

	it("consumes unknown references and unknown sources without starving later changes", function()
		local instanceMap = InstanceMap.new()
		local pointer = Instance.new("ObjectValue")
		local target = Instance.new("Part")
		local unknown = Instance.new("Folder")
		pointer.Value = target
		instanceMap:insert("POINTER", pointer)
		local changes = { [pointer] = { Value = true, Name = true }, [unknown] = { Name = true } }
		local ok, patch = pcall(createPatchSet, instanceMap, changes)
		if not ok then
			instanceMap:stop()
			pointer:Destroy()
			target:Destroy()
			unknown:Destroy()
			error(tostring(patch))
		end
		expect(#patch.updated).to.equal(1)
		expect(patch.updated[1].changedName).to.equal(pointer.Name)
		expect(patch.updated[1].changedProperties.Value).to.equal(nil)
		expect(next(changes)).to.equal(nil)
		pointer.Value = nil
		local later = createPatchSet(instanceMap, { [pointer] = { Value = true } })
		expect(later.updated[1].changedProperties.Value.Ref).to.equal("00000000000000000000000000000000")
		instanceMap:stop()
		pointer:Destroy()
		target:Destroy()
		unknown:Destroy()
	end)
	it("discards an unknown source after one attempted flush", function()
		local unknown = Instance.new("Folder")
		local changes = { [unknown] = { Name = true } }
		local patch = createPatchSet(InstanceMap.new(), changes)
		unknown:Destroy()
		expect(PatchSet.isEmpty(patch)).to.equal(true)
		expect(next(changes)).to.equal(nil)
	end)
end
