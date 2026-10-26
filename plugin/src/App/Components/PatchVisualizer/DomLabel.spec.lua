return function()
	local Rojo = script:FindFirstAncestor("Rojo")
	local Plugin = Rojo.Plugin
	local Roact = require(Rojo.Packages.Roact)
	local Flipper = require(Rojo.Packages.Flipper)
	local Theme = require(Plugin.App.Theme)
	local Tooltip = require(Plugin.App.Components.Tooltip)
	local DomLabel = require(script.Parent.DomLabel)

	local function fixture(onUpdate)
		local scene = Instance.new("ScreenGui")
		scene.Name = "Rojo759Test"
		scene.Parent = game:GetService("CoreGui")
		local height, setHeight = Roact.createBinding(24)
		local handle, theme, label
		local function unmount()
			if handle then
				local old = handle
				handle = nil
				Roact.unmount(old)
			end
		end
		local element = Roact.createElement(Theme.StudioProvider, nil, {
			Tooltips = Roact.createElement(Tooltip.Provider, nil, {
				Theme = Roact.createElement(Theme.Consumer, {
					render = function(value)
						theme = value
						return Roact.createElement("Frame", { Size = UDim2.fromOffset(320, 180) }, {
							Row = Roact.createElement(DomLabel, {
								name = "Animated",
								className = "Folder",
								patchType = "Edit",
								transparency = Roact.createBinding(0),
								depth = 1,
								depthsComplete = {},
								elementIndex = 1,
								elementHeight = height,
								setElementHeight = setHeight,
								updateEvent = {
									Fire = function()
										if onUpdate then
											onUpdate(unmount)
										end
									end,
								},
								changeList = { { "Property", "Old", "New" }, { "Name", "Before", "After" } },
							}),
						})
					end,
				}),
			}),
		})
		local originalInit = DomLabel.init
		DomLabel.init = function(self)
			originalInit(self)
			label = self
		end
		local ok, result = pcall(Roact.mount, element, scene)
		DomLabel.init = originalInit
		if not ok then
			scene:Destroy()
			error(result)
		end
		handle = result
		return label,
			unmount,
			function()
				unmount()
				label.motor:stop()
				scene:Destroy()
			end,
			function()
				return theme
			end
	end

	it("does not update an unmounted animated virtual row", function()
		local label, unmount, cleanup = fixture()
		label.motor:setGoal(Flipper.Spring.new(100))
		label.motor:stop()
		unmount()
		local ok, err = pcall(function()
			label.motor:step(1 / 60)
		end)
		cleanup()
		assert(ok, tostring(err))
	end)

	it("continues to animate and expand a mounted row", function()
		local label, _, cleanup = fixture()
		label.motor:setGoal(Flipper.Spring.new(100))
		label.motor:stop()
		local ok, err = pcall(function()
			label.motor:step(0.1)
		end)
		local height = label.props.elementHeight:getValue()
		local expanded = label.state.renderExpansion
		cleanup()
		assert(ok, tostring(err))
		expect(height > 24).to.equal(true)
		expect(expanded).to.equal(true)
	end)

	it("does not restart an animation from a click delayed past virtualization", function()
		local label, unmount, cleanup, getTheme = fixture()
		local themed = label:render()
		local frame = themed.props.render(getTheme())
		local click = frame.props[Roact.Children].Button.props[Roact.Event.Activated]
		local clickError
		task.spawn(function()
			local ok, err = pcall(click, nil, nil, 0)
			if not ok then
				clickError = err
			end
		end)
		unmount()
		task.wait(0.3)
		local restarted = label.motor._connection ~= nil
		cleanup()
		assert(clickError == nil, tostring(clickError))
		expect(restarted).to.equal(false)
	end)

	it("stops an animation when scrolling removes the row during its size update", function()
		local label, _, cleanup = fixture(function(unmount)
			unmount()
		end)
		label.motor:setGoal(Flipper.Spring.new(100))
		label.motor:stop()
		local ok, err = pcall(function()
			label.motor:step(1 / 60)
		end)
		cleanup()
		assert(ok, tostring(err))
	end)
end
