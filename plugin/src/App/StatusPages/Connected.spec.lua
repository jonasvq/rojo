return function()
	local Rojo = script:FindFirstAncestor("Rojo")
	local Plugin = Rojo.Plugin
	local Roact = require(Rojo.Packages.Roact)
	local Theme = require(Plugin.App.Theme)
	local Tooltip = require(Plugin.App.Components.Tooltip)
	local Connected = require(script.Parent.Connected)

	it("stacks version and last-update labels in narrow panels and returns to one row", function()
		local scene = Instance.new("ScreenGui")
		scene.Name = "Rojo1158Test"
		scene.Parent = game:GetService("CoreGui")
		local viewer = {
			props = { transparency = Roact.createBinding(0) },
			state = { projectName = "Example", address = "localhost:34872" },
			changeInfoText = Roact.createBinding("59 minutes ago"),
		}
		local element = Roact.createElement(Theme.StudioProvider, nil, {
			Tooltips = Roact.createElement(Tooltip.Provider, nil, {
				Theme = Roact.createElement(Theme.Consumer, {
					render = function(theme)
						local page = Connected.render(viewer)
						local fragment = page.props.render(theme)
						return Roact.createElement(
							"Frame",
							{ Size = UDim2.fromOffset(220, 200) },
							{ Heading = fragment.elements.Heading }
						)
					end,
				}),
			}),
		})
		local handle = Roact.mount(element, scene)
		local version = scene:FindFirstChild("VersionText", true)
		local info = scene:FindFirstChild("ChangeInfo", true)
		local panel = info.Parent.Parent
		local ok, err = pcall(function()
			for _, variant in { { version.Text, 220, 500 }, { "7.7.1-alpha.10+local.fork", 326, 600 } } do
				version.Text = variant[1]
				for _, width in { variant[2], variant[3], variant[2] } do
					panel.Size = UDim2.fromOffset(width, 200)
					task.wait(0.05)
					local versionRight = version.AbsolutePosition.X + version.AbsoluteSize.X
					local versionBottom = version.AbsolutePosition.Y + version.AbsoluteSize.Y
					if width == variant[2] then
						assert(
							info.AbsolutePosition.Y >= versionBottom,
							string.format(
								"labels overlap: version x=%g..%g y=%g..%g, update x=%g y=%g",
								version.AbsolutePosition.X,
								versionRight,
								version.AbsolutePosition.Y,
								versionBottom,
								info.AbsolutePosition.X,
								info.AbsolutePosition.Y
							)
						)
					else
						expect(math.abs(info.AbsolutePosition.Y - version.AbsolutePosition.Y) < 8).to.equal(true)
						expect(info.AbsolutePosition.X >= versionRight + 8).to.equal(true)
					end
					expect(info.AbsolutePosition.X >= panel.AbsolutePosition.X).to.equal(true)
					expect(info.AbsolutePosition.X + info.AbsoluteSize.X <= panel.AbsolutePosition.X + width + 1).to.equal(
						true
					)
				end
			end
		end)
		Roact.unmount(handle)
		scene:Destroy()
		assert(ok, tostring(err))
	end)
end
