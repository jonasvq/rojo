return function()
	local applyPatch = require(script.Parent.applyPatch)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local encodePatchUpdate = require(script.Parent.Parent.ChangeBatcher.encodePatchUpdate)

	it("preserves baked exported geometry on initial sync, complete updates and reconnect", function()
		local native, host = Instance.new("Model"), Instance.new("Folder")
		local pole, flag = Instance.new("Part"), Instance.new("Part")
		pole.Name, pole.Size, pole.CFrame =
			"Pole", Vector3.new(6, 0.2, 0.2), CFrame.new(0, 3, 0) * CFrame.Angles(0, 0, math.pi / 2)
		flag.Name, flag.Size, flag.CFrame = "Flag", Vector3.new(1.2, 0.7, 0.06), CFrame.new(0.72, 5.4, 0)
		pole.Anchored, flag.Anchored, pole.Parent, flag.Parent = true, true, native, native
		native.PrimaryPart = pole
		native:ScaleTo(2)
		local sourceMap, map = InstanceMap.new(), InstanceMap.new()
		sourceMap:insert("MODEL", native)
		sourceMap:insert("POLE", pole)
		sourceMap:insert("FLAG", flag)
		map:insert("ROOT", host)
		local function records()
			local result = {}
			for id, instance in sourceMap.fromIds do
				local properties = if id == "MODEL"
					then { Scale = true, PrimaryPart = true }
					else { Size = true, CFrame = true, Anchored = true }
				result[id] = {
					Id = id,
					Parent = if id == "MODEL" then "ROOT" else "MODEL",
					Name = instance.Name,
					ClassName = instance.ClassName,
					Properties = encodePatchUpdate(instance, id, properties, sourceMap).changedProperties,
					Children = if id == "MODEL" then { "POLE", "FLAG" } else {},
				}
			end
			return result
		end
		local function compare(target)
			assert(math.abs(target.fromIds.MODEL:GetScale() - native:GetScale()) < 0.0001, "stored scale changed")
			assert((target.fromIds.POLE.Size - pole.Size).Magnitude < 0.001, "exported geometry was rescaled")
			assert((target.fromIds.FLAG.Position - flag.Position).Magnitude < 0.001, "exported offset changed")
			expect(target.fromIds.MODEL.PrimaryPart).to.equal(target.fromIds.POLE)
		end
		local ok, err = pcall(function()
			local patch = PatchSet.newEmpty()
			patch.added = records()
			expect(PatchSet.isEmpty(applyPatch(map, patch))).to.equal(true)
			compare(map)
			local original = map.fromIds.POLE
			for _, scale in { 3, 1, 2 } do
				native:ScaleTo(scale)
				local values = records()
				patch = PatchSet.newEmpty()
				-- The server publishes parent updates before descendants. Export
				-- includes both the stored factor and its baked geometry.
				for _, id in { "MODEL", "POLE", "FLAG" } do
					table.insert(patch.updated, { id = id, changedProperties = values[id].Properties })
				end
				expect(PatchSet.isEmpty(applyPatch(map, patch))).to.equal(true)
				compare(map)
				expect(map.fromIds.POLE).to.equal(original)
			end
			local fresh = InstanceMap.new()
			fresh:insert("ROOT", host)
			patch = PatchSet.newEmpty()
			patch.added = records()
			local reconnectOk, reconnectErr = pcall(function()
				expect(PatchSet.isEmpty(applyPatch(fresh, patch))).to.equal(true)
				compare(fresh)
			end)
			fresh:stop()
			assert(reconnectOk, tostring(reconnectErr))
		end)
		sourceMap:stop()
		map:stop()
		native:Destroy()
		host:Destroy()
		assert(ok, tostring(err))
	end)
end
