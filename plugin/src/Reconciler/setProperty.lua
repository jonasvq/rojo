--[[
	Attempts to set a property on the given instance.
]]

local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)
local RbxDom = require(Packages.RbxDom)
local Error = require(script.Parent.Error)

local function setMesh(instance, propertyName, value)
	local mesh
	local success, err = pcall(function()
		local content = propertyName == "MeshId" and (value == "" and Content.none or Content.fromUri(value)) or value
		if instance.MeshContent == content then
			return
		end
		-- Mesh content is immutable through assignment. ApplyMesh preserves the
		-- tracked instance and its children, unlike replacing the MeshPart.
		if content == Content.none then
			mesh = Instance.new("MeshPart")
		else
			mesh = game:GetService("AssetService"):CreateMeshPartAsync(content, {
				CollisionFidelity = instance.CollisionFidelity,
				RenderFidelity = instance.RenderFidelity,
			})
		end
		local texture = instance.TextureContent
		local collisionFidelity = instance.CollisionFidelity
		local renderFidelity = instance.RenderFidelity
		instance:ApplyMesh(mesh)
		instance.TextureContent = texture
		instance.CollisionFidelity = collisionFidelity
		instance.RenderFidelity = renderFidelity
	end)
	if mesh then
		mesh:Destroy()
	end
	if not success then
		return false,
			Error.new(Error.OtherPropertyError, {
				className = instance.ClassName,
				propertyName = propertyName,
				message = tostring(err),
			})
	end
	return true
end

local function setProperty(instance, propertyName, value)
	if instance:IsA("MeshPart") and (propertyName == "MeshContent" or propertyName == "MeshId") then
		return setMesh(instance, propertyName, value)
	end

	local descriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, propertyName)

	-- We can skip unknown properties; they're not likely reflected to Lua.
	--
	-- A good example of a property like this is `Model.ModelInPrimary`, which
	-- is serialized but not reflected to Lua.
	if descriptor == nil then
		Log.trace("Skipping unknown property {}.{}", instance.ClassName, propertyName)

		return true
	end

	if descriptor.scriptability == "None" or descriptor.scriptability == "Read" then
		return false,
			Error.new(Error.UnwritableProperty, {
				className = instance.ClassName,
				propertyName = propertyName,
			})
	end

	local writeSuccess, err = descriptor:write(instance, value)

	if not writeSuccess then
		if err.kind == RbxDom.Error.Kind.Roblox and err.extra:find("lacking permission") then
			return false,
				Error.new(Error.LackingPropertyPermissions, {
					className = instance.ClassName,
					propertyName = propertyName,
				})
		end

		return false,
			Error.new(Error.OtherPropertyError, {
				className = instance.ClassName,
				propertyName = propertyName,
			})
	end

	return true
end

return setProperty
