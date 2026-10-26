local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)
local ValueCodec = require(script.Parent.Parent.ValueCodec)

return function(instance, propertyName, propertyDescriptor, instanceMap)
	local readSuccess, readResult = propertyDescriptor:read(instance)

	if not readSuccess then
		Log.warn("Could not sync back property {}.{}: {}", instance:GetFullName(), propertyName, readResult)
		return false, readResult
	end

	local dataType = propertyDescriptor.dataType
	if dataType == "Ref" then
		if readResult == nil then
			return true, { Ref = "00000000000000000000000000000000" }
		end
		local targetId = instanceMap and instanceMap.fromInstances[readResult]
		if targetId then
			return true, { Ref = targetId }
		end
		local reason = ("Reference target %s is not tracked by Rojo; property was not sent"):format(
			readResult:GetFullName()
		)
		Log.warn("Could not sync back property {}.{}: {}", instance:GetFullName(), propertyName, reason)
		return false, reason
	end

	-- Some known value encoders throw instead of returning a failure. One
	-- unsupported property must not leave the whole batch queued every frame.
	local callSuccess, encodeSuccess, encodeResult = pcall(ValueCodec.encode, readResult, dataType)
	if not callSuccess then
		encodeResult = encodeSuccess
		encodeSuccess = false
	end

	if not encodeSuccess then
		Log.warn("Could not sync back property {}.{}: {}", instance:GetFullName(), propertyName, encodeResult)
		return false, encodeResult
	end

	return true, encodeResult
end
