return function()
	local Rojo = script:FindFirstAncestor("Rojo")
	local Roact = require(Rojo.Packages.Roact)
	local Connecting = require(script.Parent.Connecting)

	it("offers Cancel while a server connection is pending", function()
		local cancelled = 0
		local page = Connecting.render({
			props = {
				text = "Connecting to server...",
				transparency = 0,
				onCancel = function()
					cancelled += 1
				end,
			},
		})
		local frame = page.props.render({
			Font = { Thin = Font.fromEnum(Enum.Font.SourceSansLight) },
			TextSize = { Medium = 14 },
			SubTextColor = Color3.new(1, 1, 1),
		})
		local cancel = frame.props[Roact.Children].Cancel
		expect(cancel).never.to.equal(nil)
		expect(cancel.props.text).to.equal("Cancel")
		cancel.props.onClick()
		expect(cancelled).to.equal(1)
	end)
end
