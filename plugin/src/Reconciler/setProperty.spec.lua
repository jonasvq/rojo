return function()
	local setProperty = require(script.Parent.setProperty)
	local Error = require(script.Parent.Error)

	it("should accept unchanged empty MeshContent and legacy MeshId", function()
		local part = Instance.new("MeshPart")
		local contentOk = setProperty(part, "MeshContent", Content.none)
		local idOk = setProperty(part, "MeshId", "")
		part:Destroy()
		assert(contentOk and idOk, "an unchanged empty mesh must not require an asset load")
	end)

	it("should report malformed mesh content and allow later property writes", function()
		local part = Instance.new("MeshPart")
		local child = Instance.new("Attachment")
		child.Parent = part
		local ok, err = setProperty(part, "MeshContent", {})
		local laterOk = setProperty(part, "Anchored", true)
		local unchanged = part.MeshContent == Content.none and child.Parent == part and part.Anchored
		part:Destroy()
		assert(not ok, "invalid mesh content must fail")
		expect(err.kind).to.equal(Error.OtherPropertyError)
		expect(err.details.propertyName).to.equal("MeshContent")
		assert(type(err.details.message) == "string" and #err.details.message > 0, "keep the engine error")
		assert(laterOk and unchanged, "a failed mesh load must preserve the part and allow subsequent updates")
	end)
end
