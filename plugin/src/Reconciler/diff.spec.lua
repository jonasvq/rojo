return function()
	local Packages = script.Parent.Parent.Parent.Packages
	local Log = require(Packages.Log)

	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)

	local diff = require(script.Parent.diff)

	local function isEmpty(table)
		return next(table) == nil, "Table was not empty"
	end

	local function size(dict)
		local len = 0

		for _ in dict do
			len = len + 1
		end

		return len
	end

	it("should preserve the unmanaged Workspace CurrentCamera while removing other unknown children", function()
		local workspace = game:GetService("Workspace")
		local previousCamera = workspace.CurrentCamera
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Workspace",
				Name = workspace.Name,
				Properties = {},
				Children = {},
				Metadata = { ignoreUnknownInstances = false },
			},
		}
		-- Keep the test place's existing children outside this fixture's removals.
		for index, child in workspace:GetChildren() do
			local id = "EXISTING" .. index
			knownInstances:insert(id, child)
			virtualInstances.ROOT.Children[index] = id
			virtualInstances[id] = {
				ClassName = child.ClassName,
				Name = child.Name,
				Properties = {},
				Children = {},
				Metadata = { ignoreUnknownInstances = true },
			}
		end
		local camera = Instance.new("Camera")
		camera.Name = "RojoCurrentCameraRegression"
		camera.CFrame = CFrame.new(17, 23, 41)
		camera.Parent = workspace
		workspace.CurrentCamera = camera
		local unknown = Instance.new("Folder")
		unknown.Parent = workspace
		local inactiveCamera = Instance.new("Camera")
		inactiveCamera.Parent = workspace
		knownInstances:insert("ROOT", workspace)
		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")
		local applyPatch = require(script.Parent.applyPatch)
		local unapplied = applyPatch(knownInstances, patch)
		local retainedCamera = workspace.CurrentCamera == camera and camera.Parent == workspace
		local retainedView = camera.CFrame == CFrame.new(17, 23, 41)
		-- Restore Studio state even when the regression assertion fails.
		workspace.CurrentCamera = previousCamera
		camera:Destroy()
		unknown:Destroy()
		inactiveCamera:Destroy()
		assert(ok, tostring(patch))
		assert(table.find(patch.removed, unknown), "ordinary unknown children must still be removed")
		assert(table.find(patch.removed, inactiveCamera), "inactive unknown cameras must still be removed")
		assert(not table.find(patch.removed, camera), "CurrentCamera must not be removed")
		assert(retainedCamera and retainedView, "applying the connect patch must preserve camera and viewpoint")
		assert(PatchSet.isEmpty(unapplied), "the connect patch must apply fully")
	end)

	it("should still update and delete a managed CurrentCamera", function()
		local workspace = game:GetService("Workspace")
		local previousCamera = workspace.CurrentCamera
		local camera = Instance.new("Camera")
		camera.Parent = workspace
		workspace.CurrentCamera = camera
		local knownInstances = InstanceMap.new()
		knownInstances:insert("ROOT", workspace)
		knownInstances:insert("CAMERA", camera)
		local virtualInstances = {
			ROOT = {
				ClassName = "Workspace",
				Name = workspace.Name,
				Properties = {},
				Children = { "CAMERA" },
				Metadata = { ignoreUnknownInstances = true },
			},
			CAMERA = {
				ClassName = "Camera",
				Name = camera.Name,
				Properties = { FieldOfView = { Float32 = 42 } },
				Children = {},
			},
		}
		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")
		virtualInstances.ROOT.Children = {}
		virtualInstances.CAMERA = nil
		local deleteOk, deletePatch = diff(knownInstances, virtualInstances, "ROOT")
		workspace.CurrentCamera = previousCamera
		camera:Destroy()
		assert(ok, tostring(patch))
		expect(#patch.updated).to.equal(1)
		expect(patch.updated[1].id).to.equal("CAMERA")
		expect(patch.updated[1].changedProperties.FieldOfView.Float32).to.equal(42)
		assert(deleteOk, tostring(deletePatch))
		expect(deletePatch.removed[1]).to.equal("CAMERA")
	end)

	it("should generate an empty patch for empty instances", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Some Name",
				Properties = {},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		rootInstance.Name = "Some Name"
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
	end)

	it("should generate a patch with a changed name", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Some Name",
				Properties = {},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		expect(#patch.updated).to.equal(1)

		local update = patch.updated[1]
		expect(update.id).to.equal("ROOT")
		expect(update.changedName).to.equal("Some Name")
		assert(isEmpty(update.changedProperties))
	end)

	it("should generate a patch with a changed property", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "StringValue",
				Name = "Value",
				Properties = {
					Value = {
						String = "Hello, world!",
					},
				},
				Children = {},
			},
		}

		local rootInstance = Instance.new("StringValue")
		rootInstance.Value = "Initial Value"
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		expect(#patch.updated).to.equal(1)

		local update = patch.updated[1]
		expect(update.id).to.equal("ROOT")
		expect(update.changedName).to.equal(nil)
		expect(size(update.changedProperties)).to.equal(1)

		local patchProperty = update.changedProperties["Value"]
		expect(patchProperty).to.be.a("table")
		local ty, value = next(patchProperty)
		expect(ty).to.equal("String")
		expect(value).to.equal("Hello, world!")
	end)

	it("should generate an empty patch if no properties changed", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "StringValue",
				Name = "Value",
				Properties = {
					Value = {
						String = "Hello, world!",
					},
				},
				Children = {},
			},
		}

		local rootInstance = Instance.new("StringValue")
		rootInstance.Value = "Hello, world!"
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))
		assert(PatchSet.isEmpty(patch), "expected empty patch")
	end)

	it("should ignore unknown properties", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Folder",
				Properties = {
					FAKE_PROPERTY = {
						String = "Hello, world!",
					},
				},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
	end)

	--[[
		Because rbx_dom_lua resolves non-canonical properties to their canonical
		variants, this test does not work as intended.

		Instead, heat_xml is diffed with Heat, the canonical property variant,
		and a patch trying to assign to heat_xml is generated. This is
		incorrect, but will require more invasive changes to fix later.
	]]
	itFIXME("should ignore unreadable properties", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Fire",
				Name = "Fire",
				Properties = {
					-- heat_xml is a serialization-only property that is not
					-- exposed to Lua.
					heat_xml = {
						Float32 = 5,
					},
				},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Fire")
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		Log.warn("{:#?}", patch)

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
	end)

	it("should generate a patch removing unknown children by default", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Folder",
				Properties = {},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		local unknownChild = Instance.new("Folder")
		unknownChild.Parent = rootInstance
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
		expect(#patch.removed).to.equal(1)
		expect(patch.removed[1]).to.equal(unknownChild)
	end)

	it("should generate an empty patch if unknown children should be ignored", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Folder",
				Properties = {},
				Children = {},
				Metadata = {
					ignoreUnknownInstances = true,
				},
			},
		}

		local rootInstance = Instance.new("Folder")
		local unknownChild = Instance.new("Folder")
		unknownChild.Parent = rootInstance
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
		assert(isEmpty(patch.removed))
	end)

	it("should generate a patch with an added child", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Folder",
				Properties = {},
				Children = { "CHILD" },
			},

			CHILD = {
				ClassName = "Folder",
				Name = "Child",
				Properties = {},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.updated))
		assert(isEmpty(patch.removed))
		expect(size(patch.added)).to.equal(1)
		expect(patch.added["CHILD"]).to.equal(virtualInstances["CHILD"])
	end)
end
