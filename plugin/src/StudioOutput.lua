local LogService = game:GetService("LogService")
local Log = require(script.Parent.Parent.Packages.Log)

local names = {
	[Enum.MessageType.MessageError] = "Error",
	[Enum.MessageType.MessageWarning] = "Warning",
	[Enum.MessageType.MessageInfo] = "Info",
	[Enum.MessageType.MessageOutput] = "Output",
}
local supported = { Error = true, Warning = true, Info = true, Output = true }
local ownTags = { "[Rojo-Trace]", "[Rojo-Info]", "[Rojo-Debug]", "[Rojo-Warn]" }
local StudioOutput = {}
StudioOutput.__index = StudioOutput

function StudioOutput.new(api, levels)
	local enabled = {}
	for _, level in levels do
		assert(supported[level], "Unknown Studio output level")
		enabled[level] = true
	end
	return setmetatable({
		__api = api,
		__enabled = enabled,
		__queue = {},
		__generation = 0,
		__dropped = 0,
		__started = false,
		__inflight = false,
	}, StudioOutput)
end

function StudioOutput.fromServerInfo(api, info)
	if type(info.capabilities) ~= "table" or not table.find(info.capabilities, "studioOutputV1") then
		return nil
	end
	local levels = info.outputLogs
	if type(levels) ~= "table" or #levels == 0 or #levels > 4 then
		return nil
	end
	for i = 1, #levels do
		if not supported[levels[i]] then
			return nil
		end
	end
	for key, level in levels do
		if type(key) ~= "number" or key % 1 ~= 0 or key < 1 or key > #levels or not supported[level] then
			return nil
		end
	end
	return StudioOutput.new(api, levels)
end

function StudioOutput:start()
	if self.__started then
		return
	end
	self.__started = true
	self.__generation += 1
	self.__connection = LogService.MessageOut:Connect(function(message, kind)
		self:__capture(message, kind)
	end)
end

function StudioOutput:stop()
	if not self.__started then
		return
	end
	self.__started = false
	self.__generation += 1
	if self.__connection then
		self.__connection:Disconnect()
		self.__connection = nil
	end
	if self.__timer then
		task.cancel(self.__timer)
		self.__timer = nil
	end
	if self.__request then
		self.__request:cancel()
		self.__request = nil
	end
	self.__inflight = false
	table.clear(self.__queue)
	self.__pendingLevel = nil
	self.__stackLevel = nil
	self.__stackCount = 0
	self.__dropped = 0
end

function StudioOutput:__schedule()
	if not self.__started or self.__timer or self.__inflight or #self.__queue == 0 then
		return
	end
	local generation = self.__generation
	self.__timer = task.delay(0.1, function()
		self.__timer = nil
		self:__flush(generation)
	end)
end

function StudioOutput:__capture(message, kind)
	if not self.__started or type(message) ~= "string" or not utf8.len(message) then
		return
	end
	local level = names[kind]
	if not level then
		return
	end
	local trimmed = message:match("^%s*(.-)%s*$")
	for _, tag in ownTags do
		if trimmed:sub(1, #tag) == tag then
			return
		end
	end
	local isFrame = trimmed:sub(1, 8) == "Script '"
	if self.__pendingTime and os.clock() - self.__pendingTime > 1 then
		self.__pendingLevel = nil
		self.__stackLevel = nil
	end
	if trimmed == "Stack Begin" and self.__pendingLevel and os.clock() - self.__pendingTime <= 1 then
		self.__stackLevel = if self.__enabled.Info then "Info" else self.__pendingLevel
		self.__stackCount = 0
	end
	local inStack = self.__stackLevel and (trimmed == "Stack Begin" or trimmed == "Stack End" or isFrame)
	if inStack then
		level = self.__stackLevel
		self.__stackCount += 1
		if trimmed == "Stack End" or self.__stackCount >= 64 then
			self.__stackLevel = nil
			self.__pendingLevel = nil
		end
	else
		self.__stackLevel = nil
		self.__pendingLevel = nil
		if (level == "Error" or level == "Warning") and self.__enabled[level] then
			self.__pendingLevel = level
			self.__pendingTime = os.clock()
		end
	end
	if not self.__enabled[level] then
		return
	end
	if #message > 8192 then
		local suffix = " [truncated]"
		local boundary = utf8.offset(message, 0, 8192 - #suffix + 1)
		message = message:sub(1, boundary - 1) .. suffix
	end
	if #self.__queue == 256 then
		table.remove(self.__queue, 1)
		self.__dropped += 1
	end
	table.insert(self.__queue, { level = level, message = message })
	self:__schedule()
end

function StudioOutput:__flush(generation)
	if not self.__started or generation ~= self.__generation or self.__inflight or #self.__queue == 0 then
		return
	end
	local batch = {}
	for _ = 1, math.min(64, #self.__queue) do
		table.insert(batch, table.remove(self.__queue, 1))
	end
	if self.__dropped > 0 then
		Log.warn("Studio output queue dropped {} older messages", self.__dropped)
		self.__dropped = 0
	end
	if not self.__started or generation ~= self.__generation then
		return
	end
	self.__inflight = true
	local function complete(ok, err)
		if generation ~= self.__generation or not self.__started then
			return
		end
		self.__request = nil
		self.__inflight = false
		if not ok then
			Log.warn("Could not forward Studio output batch: {}", tostring(err))
		end
		self:__schedule()
	end
	local ok, request = pcall(self.__api.output, self.__api, batch)
	if not ok then
		complete(false, request)
		return
	end
	if not self.__started or generation ~= self.__generation then
		request:cancel()
		return
	end
	self.__request = request
	request:andThen(function()
		complete(true)
	end, function(err)
		complete(false, err)
	end)
end

return StudioOutput
