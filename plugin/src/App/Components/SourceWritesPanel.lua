local Rojo = script:FindFirstAncestor("Rojo")
local Roact = require(Rojo.Packages.Roact)
local Theme = require(Rojo.Plugin.App.Theme)
local StudioPluginGui = require(script.Parent.Studio.StudioPluginGui)
local e = Roact.createElement

local Panel = Roact.Component:extend("SourceWritesPanel")

function Panel:init()
	self:setState({ summary = { pending = 0, issues = {} }, open = false, message = "" })
end

function Panel:attach()
	if self.unsubscribe then
		self.unsubscribe()
		self.unsubscribe = nil
	end
	self.controller = self.props.session and self.props.session.__sourceWrites
	if self.controller then
		self.unsubscribe = self.controller:observe(function(summary)
			if not self.unmounted then
				self:setState({ summary = summary })
			end
		end)
	end
end

function Panel:didMount()
	self:attach()
end

function Panel:didUpdate(previous)
	if previous.session ~= self.props.session then
		self:attach()
	end
end

function Panel:willUnmount()
	self.unmounted = true
	if self.unsubscribe then
		self.unsubscribe()
	end
end

function Panel:run(action)
	if not self.controller then
		return
	end
	self:setState({ message = "" })
	action():catch(function(reason)
		if not self.unmounted then
			self:setState({ message = tostring(reason):sub(1, 512) })
		end
	end)
end

function Panel:review(id)
	self:run(function()
		return self.controller:review(id):andThen(function(sources)
			if not self.unmounted and self.props.onReview then
				self.props.onReview(sources.studio, sources.file)
			end
		end)
	end)
end

function Panel:render()
	if not self.props.session or not self.props.session.__twoWaySync then
		return nil
	end
	local summary = self.state.summary
	local title = if self.controller
		then ("Source writes: %d pending, %d need review"):format(summary.pending, #summary.issues)
		else "Source writes: queued only; file persistence is unconfirmed"
	return Theme.with(function(theme)
		local rows = {
			Layout = e("UIListLayout", { Padding = UDim.new(0, 12), SortOrder = Enum.SortOrder.LayoutOrder }),
			Explanation = e("TextLabel", {
				Text = "Conflicting local edits stay in Studio until you choose a resolution. Confirmed writes include file readback; other editors can still race a write.",
				RichText = false,
				TextWrapped = true,
				FontFace = theme.Font.Main,
				TextSize = theme.TextSize.Body,
				TextColor3 = theme.TextColor,
				BackgroundTransparency = 1,
				Size = UDim2.new(1, -12, 0, 0),
				AutomaticSize = Enum.AutomaticSize.Y,
				LayoutOrder = 0,
			}),
			Message = e("TextLabel", {
				Text = self.state.message,
				TextWrapped = true,
				FontFace = theme.Font.Main,
				TextSize = theme.TextSize.Body,
				TextColor3 = theme.Diff.Warning,
				BackgroundTransparency = 1,
				Size = UDim2.new(1, -12, 0, 0),
				AutomaticSize = Enum.AutomaticSize.Y,
				LayoutOrder = -1,
			}),
		}
		for index = 1, math.min(#summary.issues, 8) do
			local issue = summary.issues[index]
			local function button(text, enabled, action, order)
				return e("TextButton", {
					Text = text,
					TextWrapped = true,
					FontFace = theme.Font.Main,
					TextSize = theme.TextSize.Body,
					TextColor3 = if enabled then theme.TextColor else theme.SubTextColor,
					BackgroundTransparency = 0.85,
					Size = UDim2.new(1, -12, 0, 32),
					LayoutOrder = order,
					[Roact.Event.Activated] = function()
						if enabled then
							action()
						end
					end,
				})
			end
			rows[issue.id] = e("Frame", {
				BackgroundTransparency = 1,
				Size = UDim2.new(1, 0, 0, 0),
				AutomaticSize = Enum.AutomaticSize.Y,
				LayoutOrder = index,
			}, {
				Layout = e("UIListLayout", { Padding = UDim.new(0, 5), SortOrder = Enum.SortOrder.LayoutOrder }),
				Reason = e("TextLabel", {
					Text = issue.path .. (if issue.sourcePath and issue.sourcePath ~= ""
						then "\n" .. issue.sourcePath
						else "") .. "\n" .. issue.reason,
					RichText = false,
					TextWrapped = true,
					TextXAlignment = Enum.TextXAlignment.Left,
					FontFace = theme.Font.Main,
					TextSize = theme.TextSize.Body,
					TextColor3 = theme.Diff.Warning,
					BackgroundTransparency = 1,
					Size = UDim2.new(1, -12, 0, 0),
					AutomaticSize = Enum.AutomaticSize.Y,
					LayoutOrder = 0,
				}),
				Review = button("Review both sources", not issue.running, function()
					self:review(issue.id)
				end, 1),
				KeepStudio = button(
					"Keep reviewed Studio source in the file",
					not issue.running and issue.reviewed,
					function()
						self:run(function()
							return self.controller:resolve(issue.id, "studio")
						end)
					end,
					2
				),
				UseFile = button("Use current file source in Studio", not issue.running, function()
					self:run(function()
						return self.controller:resolve(issue.id, "file")
					end)
				end, 3),
			})
		end
		if #summary.issues > 8 then
			rows.More = e("TextLabel", {
				Text = ("Showing 8 of %d conflicts; resolving them reveals the next sources."):format(#summary.issues),
				TextWrapped = true,
				FontFace = theme.Font.Main,
				TextSize = theme.TextSize.Body,
				TextColor3 = theme.TextColor,
				BackgroundTransparency = 1,
				Size = UDim2.new(1, -12, 0, 45),
				LayoutOrder = 1000,
			})
		end
		return Roact.createFragment({
			Control = e("TextButton", {
				Text = title,
				TextWrapped = true,
				FontFace = theme.Font.Main,
				TextSize = theme.TextSize.Body,
				TextColor3 = if #summary.issues > 0 then theme.Diff.Warning else theme.TextColor,
				BackgroundTransparency = 0.9,
				Size = UDim2.new(1, 0, 0, 38),
				LayoutOrder = 8,
				[Roact.Event.Activated] = function()
					if self.controller then
						self:setState({ open = not self.state.open })
					end
				end,
			}),
			Review = e(StudioPluginGui, {
				id = "Rojo_SourceWriteReview",
				title = "Review source writes",
				active = self.state.open,
				isEphemeral = true,
				initDockState = Enum.InitialDockState.Float,
				overridePreviousState = true,
				floatingSize = Vector2.new(460, 540),
				minimumSize = Vector2.new(320, 320),
				onClose = function()
					self:setState({ open = false })
				end,
			}, {
				List = e("ScrollingFrame", {
					BackgroundTransparency = 1,
					BorderSizePixel = 0,
					Size = UDim2.new(1, -16, 1, -16),
					Position = UDim2.fromOffset(8, 8),
					CanvasSize = UDim2.new(),
					AutomaticCanvasSize = Enum.AutomaticSize.Y,
					ScrollBarThickness = 6,
				}, rows),
			}),
		})
	end)
end

return Panel
