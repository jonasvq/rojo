local Rojo = script:FindFirstAncestor("Rojo")
local Roact = require(Rojo.Packages.Roact)
local Plans = require(Rojo.Plugin.DebuggerPlans)
local TextButton = require(script.Parent.TextButton)
local StudioPluginGui = require(script.Parent.Studio.StudioPluginGui)
local e = Roact.createElement

local Panel = Roact.Component:extend("DebuggerPanel")
Panel.consentText =
	"Enable exclusive automatic Run debugging for this connection?\n\nRojo takes exclusive ownership of Studio's debugger callback. Do not share it with another debugger plugin.\n\nALL unrelated or manual stops will resume automatically. Requested breakpoint lines can replace existing breakpoints; their previous configuration cannot be restored.\n\nOnly combined local Run is supported. Rojo never holds a pause for external input and never starts Run or Play. Disable this mode to use ordinary Studio debugging."

function Panel:init()
	self:setState({ prompt = false, status = "Disabled" })
end

function Panel:willUnmount()
	self.unmounting = true
	if self.controller then
		self.controller:stop()
	end
end

function Panel:didUpdate(previous)
	if previous.session ~= self.props.session then
		if self.controller then
			self.controller:stop()
			self.controller = nil
		end
		self:setState({ prompt = false, status = "Disabled" })
	end
end

function Panel:accept()
	local session = self.props.session
	if not self.state.prompt or not Plans.supported(session) or not session.__apiContext.__socketOpen then
		self:setState({ prompt = false, status = "Wait for initial sync and socket readiness" })
		return
	end
	self:setState({ prompt = false })
	if not self.controller then
		self.controller = Plans.new(session, function(state)
			if not self.unmounting then
				self:setState({ status = state })
			end
		end)
		session.__debuggerPlans = self.controller
	end
	local ok, err = pcall(self.controller.enable, self.controller)
	if not ok then
		self.controller:stop()
		self:setState({ status = tostring(err):sub(1, 256) })
	end
end

function Panel:toggle()
	if self.controller and self.controller.enabled then
		self.controller:stop()
	else
		self:setState({ prompt = true })
	end
end

function Panel:render()
	if not Plans.supported(self.props.session) then
		return nil
	end
	return Roact.createFragment({
		Control = e("Frame", { BackgroundTransparency = 1, Size = UDim2.new(1, 0, 0, 55), LayoutOrder = 4 }, {
			Button = e(TextButton, {
				text = if self.controller and self.controller.enabled
					then "Disable automatic debugger"
					else "Automatic debugger…",
				style = "Bordered",
				enabled = true,
				transparency = self.props.transparency,
				onClick = function()
					self:toggle()
				end,
			}),
			Status = e("TextLabel", {
				Text = self.state.status,
				TextSize = 12,
				TextWrapped = true,
				TextColor3 = Color3.fromRGB(180, 180, 180),
				BackgroundTransparency = 1,
				Size = UDim2.new(1, 0, 0, 22),
				Position = UDim2.fromOffset(0, 33),
			}),
		}),
		Consent = e(StudioPluginGui, {
			id = "Rojo_AutomaticDebuggerConsent",
			title = "Automatic debugger consent",
			active = self.state.prompt,
			isEphemeral = true,
			initDockState = Enum.InitialDockState.Float,
			overridePreviousState = true,
			floatingSize = Vector2.new(430, 410),
			minimumSize = Vector2.new(360, 380),
			onClose = function()
				self:setState({ prompt = false })
			end,
		}, {
			Body = e("TextLabel", {
				Text = Panel.consentText,
				RichText = false,
				TextWrapped = true,
				TextSize = 16,
				Font = Enum.Font.SourceSans,
				TextColor3 = Color3.new(1, 1, 1),
				BackgroundColor3 = Color3.fromRGB(35, 35, 35),
				Size = UDim2.new(1, -20, 1, -95),
				Position = UDim2.fromOffset(10, 10),
			}),
			Accept = e("TextButton", {
				Text = "Enable exclusive automatic debugger",
				TextSize = 15,
				Size = UDim2.new(1, -20, 0, 30),
				Position = UDim2.new(0, 10, 1, -75),
				[Roact.Event.Activated] = function()
					self:accept()
				end,
			}),
			Cancel = e("TextButton", {
				Text = "Cancel",
				TextSize = 15,
				Size = UDim2.new(1, -20, 0, 30),
				Position = UDim2.new(0, 10, 1, -40),
				[Roact.Event.Activated] = function()
					self:setState({ prompt = false })
				end,
			}),
		}),
	})
end

return Panel
