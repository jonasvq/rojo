local Roact = require(script:FindFirstAncestor("Rojo").Packages.Roact)
local e = Roact.createElement

return function(props)
	local state = props.state or { grants = {} }
	if not props.open and not state.prompt then
		return nil
	end
	local children = {
		Layout = e("UIListLayout", { Padding = UDim.new(0, 6), SortOrder = Enum.SortOrder.LayoutOrder }),
		Padding = e("UIPadding", {
			PaddingTop = UDim.new(0, 8),
			PaddingLeft = UDim.new(0, 8),
			PaddingRight = UDim.new(0, 8),
			PaddingBottom = UDim.new(0, 8),
		}),
	}
	local order = 0
	local function row(text, callback, color)
		order += 1
		children[tostring(order)] = e(if callback then "TextButton" else "TextLabel", {
			Text = text,
			RichText = false,
			TextWrapped = true,
			TextSize = 14,
			TextColor3 = Color3.new(1, 1, 1),
			BackgroundColor3 = color or Color3.fromRGB(50, 50, 50),
			Size = UDim2.new(1, 0, 0, 0),
			AutomaticSize = Enum.AutomaticSize.Y,
			Font = Enum.Font.SourceSans,
			LayoutOrder = order,
			[Roact.Event.Activated] = if callback then callback else nil,
		})
	end
	row("Rojo — Plugin API access")
	row(state.status or "Discovery unavailable")
	row("Grants last until revoked, plugin unload or Rojo reload. Plugin names are labels.")
	if state.prompt then
		local prompt = state.prompt
		row("Request from plugin: " .. prompt.label)
		for _, name in prompt.names do
			local choice = prompt.decisions[name]
			if choice == nil then
				row("Allow " .. name, function()
					prompt.decide(name, true)
				end, Color3.fromRGB(40, 90, 60))
				row("Deny " .. name, function()
					prompt.decide(name, false)
				end, Color3.fromRGB(100, 45, 45))
			else
				row(name .. ": " .. (if choice then "Allowed" else "Denied"))
			end
		end
		row("Deny remaining / close request", prompt.deny)
	end
	for _, grant in state.grants do
		row(grant.label .. "\n" .. table.concat(grant.names, ", ") .. "\nRevoke access", grant.revoke)
	end
	if #state.grants == 0 then
		row("No active grants")
	end
	row("Close", function()
		if state.prompt then
			state.prompt.deny()
		end
		props.onClose()
	end)
	return e("ScrollingFrame", {
		Size = UDim2.new(0, 360, 0, 420),
		Position = UDim2.new(1, -372, 0, 12),
		BackgroundColor3 = Color3.fromRGB(30, 30, 30),
		BorderSizePixel = 0,
		CanvasSize = UDim2.new(),
		AutomaticCanvasSize = Enum.AutomaticSize.Y,
		ScrollBarThickness = 6,
		ZIndex = 10,
	}, children)
end
