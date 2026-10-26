return function()
	local decodeValue = require(script.Parent.decodeValue)
	local trueEquals = require(script.Parent.trueEquals)
	local encodeProperty = require(script.Parent.Parent.ChangeBatcher.encodeProperty)
	local encoded = {
		TweenInfo = {
			time = 0.2,
			delayTime = 0.1,
			easingStyle = 3,
			easingDirection = 0,
			repeatCount = 2,
			reverses = false,
		},
	}
	it("decodes mixed TweenInfo attributes without dropping other values", function()
		local ok, values = decodeValue({ Attributes = { Motion = encoded, Accent = { String = "$Pink" } } }, {})
		assert(ok, tostring(values))
		assert(typeof(values.Motion) == "TweenInfo", "TweenInfo attribute was dropped")
		assert(values.Accent == "$Pink", "Ordinary attribute changed")
		assert(
			trueEquals(values.Motion, TweenInfo.new(0.2, Enum.EasingStyle.Quad, Enum.EasingDirection.In, 2, false, 0.1))
		)
	end)
	it("encodes TweenInfo attributes without silently dropping them", function()
		local instance = Instance.new("Folder")
		local descriptor = {
			dataType = "Attributes",
			read = function()
				return true,
					{
						Motion = TweenInfo.new(0.2, Enum.EasingStyle.Quad, Enum.EasingDirection.In, 2, false, 0.1),
						Accent = "$Pink",
					}
			end,
		}
		local ok, result = encodeProperty(instance, "Attributes", descriptor)
		instance:Destroy()
		assert(ok, tostring(result))
		assert(result.Attributes.Motion and result.Attributes.Motion.TweenInfo, "TweenInfo was omitted")
	end)
	it("rejects an unknown or incomplete nested attribute instead of dropping it", function()
		local ok = decodeValue(
			{ Attributes = { Motion = { TweenInfo = { time = 1 } }, Accent = { String = "$Pink" } } },
			{}
		)
		assert(not ok, "An incomplete TweenInfo was accepted")
		ok = decodeValue({ Attributes = { Future = { UnsupportedFutureType = 1 } } }, {})
		assert(not ok, "An unknown attribute was silently dropped")
	end)
end
