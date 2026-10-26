return function()
	local Rojo = script:FindFirstAncestor("Rojo")
	local Roact = require(Rojo.Packages.Roact)
	local Tooltip = require(script.Parent.Tooltip)

	it("ignores a queued popup whose virtualized trigger was unmounted", function()
		local ref = Roact.createRef()
		local consumer = Tooltip.Container.render({ state = { size = Vector2.new(300, 240) } })
		local frame = consumer.props.render({
			tips = {
				queued = { Text = "Queued tooltip", Position = Vector2.zero, Trigger = ref },
			},
		})
		local popup = frame.props[Roact.Children].queued
		local themed = popup.component(popup.props)
		local ok, result = pcall(themed.props.render, {
			Font = { Main = Font.fromEnum(Enum.Font.SourceSans) },
			TextSize = { Medium = 14 },
		})
		assert(ok, tostring(result))
		expect(result).to.equal(nil)
	end)

	it("hides detached triggers but keeps popups for attached controls", function()
		local parent = Instance.new("Folder")
		local trigger = Instance.new("Frame")
		trigger.Parent = parent
		local ref = {
			getValue = function()
				return trigger
			end,
		}
		local consumer = Tooltip.Container.render({ state = { size = Vector2.new(300, 240) } })
		local frame = consumer.props.render({
			tips = { tip = {
				Text = "Attached tooltip",
				Position = Vector2.zero,
				Trigger = ref,
			} },
		})
		local popup = frame.props[Roact.Children].tip
		local themed = popup.component(popup.props)
		local theme = {
			Font = { Main = Font.fromEnum(Enum.Font.SourceSans) },
			TextSize = { Medium = 14 },
			Button = { Bordered = { Enabled = { TextColor = Color3.new(1, 1, 1) } } },
			BorderedContainer = { BackgroundColor = Color3.new(), BorderColor = Color3.new() },
		}
		local visible = themed.props.render(theme)
		expect(visible).never.to.equal(nil)
		expect(visible.props.size.X.Offset > 0).to.equal(true)
		trigger.Parent = nil
		expect(themed.props.render(theme)).to.equal(nil)
		trigger:Destroy()
		parent:Destroy()
	end)
end
