local RbxDom = require(script.Parent.Parent.Packages.RbxDom)
local ValueCodec = {}

local function invoke(method, ...)
	local called, success, value = pcall(method, ...)
	if not called then
		return false, success
	end
	return success, value
end

local function enumValue(enum, value)
	for _, item in enum:GetEnumItems() do
		if item.Value == value then
			return item
		end
	end
	error("Unknown " .. tostring(enum) .. " value " .. tostring(value))
end

local function decodeTween(value)
	assert(type(value) == "table", "TweenInfo must be an object")
	for _, field in { "time", "delayTime", "easingStyle", "easingDirection", "repeatCount" } do
		local number = value[field]
		assert(
			type(number) == "number" and number == number and math.abs(number) < math.huge,
			"Invalid TweenInfo " .. field
		)
	end
	assert(type(value.reverses) == "boolean", "Invalid TweenInfo reverses")
	assert(value.repeatCount % 1 == 0, "TweenInfo repeatCount must be an integer")
	return TweenInfo.new(
		value.time,
		enumValue(Enum.EasingStyle, value.easingStyle),
		enumValue(Enum.EasingDirection, value.easingDirection),
		value.repeatCount,
		value.reverses,
		value.delayTime
	)
end

function ValueCodec.decode(encoded)
	local ty, value = next(encoded)
	if ty == "TweenInfo" then
		local ok, result = pcall(decodeTween, value)
		return ok, result
	elseif ty == "Attributes" then
		local result = {}
		for name, entry in value do
			local ok, decoded = ValueCodec.decode(entry)
			if not ok then
				return false, "Attribute " .. name .. ": " .. tostring(decoded)
			end
			result[name] = decoded
		end
		return true, result
	end
	return invoke(RbxDom.EncodedValue.decode, encoded)
end

function ValueCodec.encode(value, ty)
	if ty == "TweenInfo" then
		return true,
			{
				TweenInfo = {
					time = value.Time,
					delayTime = value.DelayTime,
					easingStyle = value.EasingStyle.Value,
					easingDirection = value.EasingDirection.Value,
					repeatCount = value.RepeatCount,
					reverses = value.Reverses,
				},
			}
	elseif ty == "Attributes" then
		local result = {}
		for name, entry in value do
			local ok, encoded
			if typeof(entry) == "TweenInfo" then
				ok, encoded = ValueCodec.encode(entry, "TweenInfo")
			else
				ok, encoded = invoke(RbxDom.EncodedValue.encodeNaive, entry)
			end
			if not ok then
				return false, "Attribute " .. name .. ": " .. tostring(encoded)
			end
			result[name] = encoded
		end
		return true, { Attributes = result }
	end
	return invoke(RbxDom.EncodedValue.encode, value, ty)
end

return ValueCodec
