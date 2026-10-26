local Packages = script.Parent.Parent.Parent.Packages
local RbxDom = require(Packages.RbxDom)
local Error = require(script.Parent.Error)
local setProperty = require(script.Parent.setProperty)

local function resetProperty(instance, propertyName)
	if instance:IsA("MeshPart") and (propertyName == "MeshContent" or propertyName == "MeshId") then
		return setProperty(instance, "MeshContent", Content.none)
	end

	local descriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, propertyName)
	local details = { className = instance.ClassName, propertyName = propertyName }
	if descriptor == nil then
		return false, Error.new(Error.UnknownProperty, details)
	end
	if descriptor.scriptability == "None" or descriptor.scriptability == "Read" then
		return false, Error.new(Error.UnwritableProperty, details)
	end

	local ok, reset, err = pcall(function()
		if descriptor.scriptability == "Custom" then
			-- Synthetic properties have descriptor writers, rather than a
			-- directly resettable engine property.
			if descriptor.name == "Attributes" or descriptor.name == "Tags" then
				return descriptor:write(instance, {})
			end
			local default = Instance.new(instance.ClassName)
			local readOk, value = descriptor:read(default)
			default:Destroy()
			if not readOk then
				return false, value
			end
			return descriptor:write(instance, value)
		end
		instance:ResetPropertyToDefault(descriptor.name)
		return true
	end)
	if not ok or not reset then
		details.message = tostring(if ok then err else reset)
		return false, Error.new(Error.OtherPropertyError, details)
	end
	return true
end

return resetProperty
