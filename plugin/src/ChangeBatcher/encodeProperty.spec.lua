return function()
	local encodeProperty = require(script.Parent.encodeProperty)

	it("reports a throwing value encoder as a failed property without escaping", function()
		local part = Instance.new("Part")
		local descriptor = {
			dataType = "Region3",
			read = function()
				return true, Region3.new(Vector3.zero, Vector3.one)
			end,
		}
		local ok, encoded = pcall(encodeProperty, part, "Region", descriptor)
		part:Destroy()
		assert(ok, tostring(encoded))
		expect(encoded).to.equal(false)
	end)
end
