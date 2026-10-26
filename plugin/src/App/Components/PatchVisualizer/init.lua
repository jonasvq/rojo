local Rojo = script:FindFirstAncestor("Rojo")
local Plugin = Rojo.Plugin
local Packages = Rojo.Packages

local Roact = require(Packages.Roact)

local PatchTree = require(Plugin.PatchTree)
local PatchSet = require(Plugin.PatchSet)

local Theme = require(Plugin.App.Theme)
local VirtualScroller = require(Plugin.App.Components.VirtualScroller)
local BorderedContainer = require(Plugin.App.Components.BorderedContainer)
local TextButton = require(Plugin.App.Components.TextButton)
local failureRows = require(script.failureRows)

local e = Roact.createElement

local DomLabel = require(script.DomLabel)

local PatchVisualizer = Roact.Component:extend("PatchVisualizer")

function PatchVisualizer:init()
	self:setState({ failuresOnly = false })
	self.contentSize, self.setContentSize = Roact.createBinding(Vector2.new(0, 0))

	self.updateEvent = Instance.new("BindableEvent")
end

function PatchVisualizer:willUnmount()
	self.updateEvent:Destroy()
end

function PatchVisualizer:shouldUpdate(nextProps, nextState)
	if self.state.failuresOnly ~= nextState.failuresOnly or self.props.unappliedPatch ~= nextProps.unappliedPatch then
		return true
	end
	if self.props.patchTree ~= nextProps.patchTree then
		return true
	end

	local currentPatch, nextPatch = self.props.patch, nextProps.patch
	if currentPatch ~= nil or nextPatch ~= nil then
		return not PatchSet.isEqual(currentPatch, nextPatch)
	end

	return false
end

function PatchVisualizer:render()
	local patchTree = self.props.patchTree
	if patchTree == nil and self.props.patch ~= nil then
		patchTree = PatchTree.build(
			self.props.patch,
			self.props.instanceMap,
			self.props.changeListHeaders or { "Property", "Current", "Incoming" }
		)
		if self.props.unappliedPatch then
			patchTree =
				PatchTree.updateMetadata(patchTree, self.props.patch, self.props.instanceMap, self.props.unappliedPatch)
		end
	end

	local rows, failureCount = failureRows(patchTree, self.state.failuresOnly)
	local headerHeight = if failureCount > 0 or self.state.failuresOnly then 40 else 0
	-- Recusively draw tree
	local scrollElements, elementHeights, elementIndex = {}, {}, 0

	if patchTree then
		local elementTotal = #rows
		local depthsComplete = {}
		local function drawNode(row)
			local node, depth = row.node, row.depth
			elementIndex += 1

			local isFinalChild = row.isFinalChild

			local elementHeight, setElementHeight = Roact.createBinding(24)
			elementHeights[elementIndex] = elementHeight
			scrollElements[elementIndex] = e(DomLabel, {
				transparency = self.props.transparency,
				showStringDiff = self.props.showStringDiff,
				showTableDiff = self.props.showTableDiff,
				updateEvent = self.updateEvent,
				elementHeight = elementHeight,
				setElementHeight = setElementHeight,
				elementIndex = elementIndex,
				isFinalElement = elementIndex == elementTotal,
				depth = depth,
				depthsComplete = table.clone(depthsComplete),
				hasChildren = row.hasChildren,
				isFinalChild = isFinalChild,
				patchType = node.patchType,
				className = node.className,
				isWarning = node.isWarning,
				instance = node.instance,
				name = node.name,
				changeInfo = node.changeInfo,
				changeList = node.changeList,
			})

			if isFinalChild then
				depthsComplete[depth] = true
			end
		end

		for _, row in rows do
			local depth = row.depth
			depthsComplete[depth] = false
			for i = depth + 1, #depthsComplete do
				depthsComplete[i] = nil
			end

			drawNode(row)
		end
	end

	return Theme.with(function(theme)
		return e(BorderedContainer, {
			transparency = self.props.transparency,
			size = self.props.size,
			position = self.props.position,
			anchorPoint = self.props.anchorPoint,
			layoutOrder = self.props.layoutOrder,
		}, {
			FailureFilter = if headerHeight > 0
				then e(TextButton, {
					text = if self.state.failuresOnly then "Show all changes" else `Failures only ({failureCount})`,
					style = "Bordered",
					enabled = true,
					transparency = self.props.transparency,
					position = UDim2.fromOffset(8, 3),
					onClick = function()
						self:setState(function(state)
							return { failuresOnly = not state.failuresOnly }
						end)
					end,
				})
				else nil,
			CleanMerge = e("TextLabel", {
				Visible = #scrollElements == 0,
				Text = if self.state.failuresOnly
					then "No failed changes in this view."
					else "No changes to sync, project is up to date.",
				FontFace = theme.Font.Main,
				TextSize = theme.TextSize.Medium,
				TextColor3 = theme.TextColor,
				TextWrapped = true,
				Size = UDim2.new(1, 0, 1, -headerHeight),
				Position = UDim2.fromOffset(0, headerHeight),
				BackgroundTransparency = 1,
			}),

			-- A different key resets scroll/expansion when switching views.
			[if self.state.failuresOnly then "FailureScroller" else "VirtualScroller"] = e(VirtualScroller, {
				size = UDim2.new(1, 0, 1, -headerHeight - 2),
				position = UDim2.new(0, 0, 0, headerHeight + 2),
				transparency = self.props.transparency,
				count = #scrollElements,
				updateEvent = self.updateEvent.Event,
				render = function(i)
					return scrollElements[i]
				end,
				getHeightBinding = function(i)
					return elementHeights[i]
				end,
			}),
		})
	end)
end

return PatchVisualizer
