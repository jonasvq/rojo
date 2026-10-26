return function()
	local Packages = script.Parent.Parent.Parent.Packages
	local Log = require(Packages.Log)
	local Fmt = require(Packages.Fmt)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local diff = require(script.Parent.diff)

	it("warns when requested Workspace settings cannot be compared or live-synced", function()
		local map = InstanceMap.new()
		local workspace = game:GetService("Workspace")
		map:insert("ROOT", workspace)
		local originalWarn = Log.warn
		local warnings = {}
		Log.warn = function(template, ...)
			table.insert(warnings, Fmt.fmt(template, ...))
		end
		local ok, err = pcall(function()
			local success, patch = diff(map, {
				ROOT = {
					ClassName = "Workspace",
					Name = workspace.Name,
					Properties = { PlayerCharacterDestroyBehavior = { Enum = 2 } },
					Children = {},
					Metadata = { ignoreUnknownInstances = true },
				},
			}, "ROOT")
			expect(success).to.equal(true)
			expect(PatchSet.isEmpty(patch)).to.equal(true)
			expect(#warnings).to.equal(1)
			expect(string.find(warnings[1], "Workspace.PlayerCharacterDestroyBehavior", 1, true) ~= nil).to.equal(true)
		end)
		Log.warn = originalWarn
		map:stop()
		assert(ok, tostring(err))
	end)

	it("continues to diff readable properties and quietly skips unknown reflection fields", function()
		local map = InstanceMap.new()
		local part = Instance.new("Part")
		map:insert("ROOT", part)
		local originalWarn = Log.warn
		local count = 0
		Log.warn = function()
			count += 1
		end
		local ok, err = pcall(function()
			local success, patch = diff(map, {
				ROOT = {
					ClassName = "Part",
					Name = part.Name,
					Properties = { Archivable = { Bool = false }, NotAReflectionProperty = { Bool = true } },
					Children = {},
				},
			}, "ROOT")
			expect(success).to.equal(true)
			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedProperties.Archivable.Bool).to.equal(false)
			expect(patch.updated[1].changedProperties.NotAReflectionProperty).to.equal(nil)
			expect(count).to.equal(0)
		end)
		Log.warn = originalWarn
		map:stop()
		part:Destroy()
		assert(ok, tostring(err))
	end)
end
