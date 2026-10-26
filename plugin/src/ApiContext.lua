local Packages = script.Parent.Parent.Packages
local HttpService = game:GetService("HttpService")
local Http = require(Packages.Http)
local Log = require(Packages.Log)
local Promise = require(Packages.Promise)

local Config = require(script.Parent.Config)
local Types = require(script.Parent.Types)
local Version = require(script.Parent.Version)
local placeIdError = require(script.Parent.placeIdError)

-- Handshake shape is a connection boundary, not optional deep typechecking.
local validateApiInfo = Types.ApiInfoResponse
local validateApiRead = Types.ifEnabled(Types.ApiReadResponse)
local validateApiSocketPacket = Types.ifEnabled(Types.ApiSocketPacket)
local validateApiSerialize = Types.ifEnabled(Types.ApiSerializeResponse)
local validateApiRefPatch = Types.ifEnabled(Types.ApiRefPatchResponse)

local function rejectFailedRequests(response)
	if response.code >= 400 then
		local message = string.format("HTTP %s:\n%s", tostring(response.code), response.body)

		return Promise.reject(message)
	end

	return response
end

local function serverConnectionError(baseUrl, details)
	return (
		"The connection to the Rojo server at %s was lost.\n"
		.. "Check the serve terminal, restart rojo serve if it stopped, then reconnect.\n\n"
		.. "Details: %s"
	):format(baseUrl, details)
end

local function rejectWrongProtocolVersion(infoResponseBody)
	if infoResponseBody.protocolVersion ~= Config.protocolVersion then
		local message = (
			"Found a Rojo dev server, but it's using a different protocol version, and is incompatible."
			.. "\nMake sure you have matching versions of both the Rojo plugin and server!"
			.. "\n\nYour client is version %s, with protocol version %s. It expects server version %s."
			.. "\nYour server is version %s, with protocol version %s."
			.. "\n\nGo to https://github.com/jonasvq/rojo for more details."
		):format(
			Version.display(Config.version),
			Config.protocolVersion,
			Config.expectedServerVersionString,
			infoResponseBody.serverVersion,
			infoResponseBody.protocolVersion
		)

		return Promise.reject(message)
	end

	return Promise.resolve(infoResponseBody)
end

local function rejectWrongPlaceId(infoResponseBody)
	local message = placeIdError(infoResponseBody, game.PlaceId)
	if message then
		return Promise.reject(message)
	end
	return Promise.resolve(infoResponseBody)
end

local ApiContext = {}
ApiContext.__index = ApiContext

function ApiContext.new(baseUrl)
	assert(type(baseUrl) == "string", "baseUrl must be a string")

	local self = {
		__baseUrl = baseUrl,
		__sessionId = nil,
		__messageCursor = -1,
		__wsClient = nil,
		__connected = true,
		__activeRequests = {},
		__socketOpen = false,
		__socketObservers = {},
	}

	return setmetatable(self, ApiContext)
end

function ApiContext:__fmtDebug(output)
	output:writeLine("ApiContext {{")
	output:indent()

	output:writeLine("Connected: {}", self.__connected)
	output:writeLine("Base URL: {}", self.__baseUrl)
	output:writeLine("Session ID: {}", self.__sessionId)
	output:writeLine("Message Cursor: {}", self.__messageCursor)

	output:unindent()
	output:write("}")
end

function ApiContext:disconnect()
	self.__connected = false
	self:__setSocketOpen(false)
	for request in self.__activeRequests do
		Log.trace("Cancelling request {}", request)
		request:cancel()
	end
	self.__activeRequests = {}

	if self.__wsClient then
		Log.trace("Closing WebSocket client")
		self.__wsClient:Close()
	end
	self.__wsClient = nil
end

function ApiContext:__observeSocket(callback)
	self.__socketObservers[callback] = true
	callback(self.__socketOpen)
	return function()
		self.__socketObservers[callback] = nil
	end
end

function ApiContext:__setSocketOpen(open)
	self.__socketOpen = open
	for callback in self.__socketObservers do
		local ok, err = pcall(callback, open)
		if not ok then
			Log.warn("Socket readiness observer failed: {}", err)
		end
	end
end

function ApiContext:setMessageCursor(index)
	self.__messageCursor = index
end

function ApiContext:connect()
	local url = ("%s/api/rojo"):format(self.__baseUrl)

	return Http.get(url)
		:andThen(rejectFailedRequests)
		:andThen(function(response)
			local success, body = pcall(Http.Response.msgpack, response)
			local valid, details = false, "Expected a MessagePack server-info response"
			if success then
				valid, details = validateApiInfo(body)
			end
			if not valid then
				return Promise.reject(
					(
						"Invalid Rojo server handshake from %s.\n"
						.. "Check that this address is running rojo serve and that the CLI and Studio plugin versions match.\nDetails: %s"
					):format(url, tostring(details))
				)
			end
			return body
		end)
		:andThen(rejectWrongProtocolVersion)
		:andThen(function(body)
			if self.__expectedSessionId ~= nil and body.sessionId ~= self.__expectedSessionId then
				error("The rojo open server session changed. Start rojo open again or connect manually.", 0)
			end

			return body
		end)
		:andThen(rejectWrongPlaceId)
		:andThen(function(body)
			self.__sessionId = body.sessionId
			self.__serializedModels = table.find(body.capabilities or {}, "serializedModelsV1") ~= nil
			self.__supportsWriteReceipts = table.find(body.capabilities or {}, "writeReceiptsV1") ~= nil
			if not table.find(body.capabilities or {}, "propertyRemovalsV1") then
				Log.warn(
					"This Rojo server cannot sync property resets. Update the server to enable removal of properties from files."
				)
			end

			return body
		end)
end

function ApiContext:read(ids)
	local url = ("%s/api/read/%s?attributeTweenInfoV1=true&classAuthorityV1=true"):format(
		self.__baseUrl,
		table.concat(ids, ",")
	)
	if self.__writeReceipts then
		url ..= "&writeReceiptsV1=true"
	end

	return Http.get(url):andThen(rejectFailedRequests):andThen(Http.Response.msgpack):andThen(function(body)
		if body.sessionId ~= self.__sessionId then
			return Promise.reject("Server changed ID")
		end

		assert(validateApiRead(body))

		return body
	end)
end

function ApiContext:__sourceRequest(route, body)
	if not self.__connected then
		return Promise.reject("Source write connection ended")
	end
	local sessionId = self.__sessionId
	if body then
		body.sessionId = sessionId
	end
	local url = self.__baseUrl .. "/api/write-receipts" .. route
	local request = if body then Http.post(url, Http.msgpackEncode(body)) else Http.get(url)
	self.__activeRequests[request] = true
	return request
		:andThen(function(response)
			local success, decoded = pcall(Http.Response.msgpack, response)
			if not success then
				return Promise.reject("Invalid source receipt response; persistence is uncertain")
			end
			if response.code >= 400 then
				return Promise.reject(tostring(decoded.details or ("HTTP " .. response.code)))
			end
			if not self.__connected or self.__sessionId ~= sessionId or decoded.sessionId ~= sessionId then
				return Promise.reject("Source receipt session changed")
			end
			return decoded
		end)
		:finally(function()
			self.__activeRequests[request] = nil
		end)
end

function ApiContext:sourceView(id)
	return self:__sourceRequest("/source/" .. id)
end

function ApiContext:submitSource(body)
	return self:__sourceRequest("", body)
end

function ApiContext:sourceReceipt(id, origin)
	return self:__sourceRequest("/" .. id .. "?sessionId=" .. self.__sessionId .. "&originId=" .. origin)
end

function ApiContext:write(patch)
	local url = ("%s/api/write"):format(self.__baseUrl)

	local updated = {}
	for _, update in ipairs(patch.updated) do
		local fixedUpdate = {
			id = update.id,
			changedName = update.changedName,
		}

		if next(update.changedProperties) ~= nil then
			fixedUpdate.changedProperties = update.changedProperties
		end

		table.insert(updated, fixedUpdate)
	end

	-- Only add the 'added' field if the table is non-empty, or else the msgpack
	-- encode implementation will turn the table into an array instead of a map,
	-- causing API validation to fail.
	local added
	if next(patch.added) ~= nil then
		added = patch.added
	end

	local body = {
		sessionId = self.__sessionId,
		removed = patch.removed,
		updated = updated,
		added = added,
	}

	body = Http.msgpackEncode(body)

	return Http.post(url, body)
		:andThen(rejectFailedRequests)
		:andThen(Http.Response.msgpack)
		:andThen(function(responseBody)
			Log.info("Write response: {:?}", responseBody)

			return responseBody
		end)
end

function ApiContext:connectWebSocket(packetHandlers)
	local url = ("%s/api/socket/%s?propertyRemovalsV1=true&attributeTweenInfoV1=true&classAuthorityV1=true"):format(
		self.__baseUrl,
		self.__messageCursor
	)
	-- Convert HTTP/HTTPS URL to WS/WSS
	url = url:gsub("^http://", "ws://"):gsub("^https://", "wss://")

	return Promise.new(function(resolve, reject)
		local success, wsClient =
			pcall(HttpService.CreateWebStreamClient, HttpService, Enum.WebStreamClientType.WebSocket, {
				Url = url,
			})
		if not success then
			reject("Failed to create WebSocket client: " .. tostring(wsClient))
			return
		end
		self.__wsClient = wsClient

		local closed, errored, received, opened
		opened = wsClient.Opened:Connect(function()
			if self.__connected and self.__wsClient == wsClient then
				self:__setSocketOpen(true)
			end
		end)

		received = self.__wsClient.MessageReceived:Connect(function(msg)
			local data = Http.msgpackDecode(msg)
			if data.sessionId ~= self.__sessionId then
				Log.warn("Received message with wrong session ID; ignoring")
				return
			end

			assert(validateApiSocketPacket(data))

			Log.trace("Received websocket packet: {:#?}", data)

			local handler = packetHandlers[data.packetType]
			if handler then
				local ok, err = pcall(handler, data.body)
				if not ok then
					Log.error("Error in WebSocket packet handler for type '%s': %s", data.packetType, err)
				end
			else
				Log.warn("No handler for WebSocket packet type '%s'", data.packetType)
			end
		end)

		closed = self.__wsClient.Closed:Connect(function()
			self:__setSocketOpen(false)
			opened:Disconnect()
			closed:Disconnect()
			errored:Disconnect()
			received:Disconnect()

			if self.__connected then
				reject(serverConnectionError(self.__baseUrl, "WebSocket connection closed unexpectedly"))
			else
				resolve()
			end
		end)

		errored = self.__wsClient.Error:Connect(function(code, msg)
			self:__setSocketOpen(false)
			opened:Disconnect()
			closed:Disconnect()
			errored:Disconnect()
			received:Disconnect()

			if self.__connected then
				reject(
					serverConnectionError(
						self.__baseUrl,
						"WebSocket error: " .. tostring(code) .. " - " .. tostring(msg)
					)
				)
			else
				resolve()
			end
		end)
		if wsClient.ConnectionState == Enum.WebStreamClientState.Open then
			self:__setSocketOpen(true)
		end
	end)
end

function ApiContext:open(id)
	local url = ("%s/api/open/%s"):format(self.__baseUrl, id)

	return Http.post(url, ""):andThen(rejectFailedRequests):andThen(Http.Response.msgpack):andThen(function(body)
		if body.sessionId ~= self.__sessionId then
			return Promise.reject("Server changed ID")
		end

		return nil
	end)
end

function ApiContext:serialize(ids: { string })
	local url = ("%s/api/serialize"):format(self.__baseUrl)
	if self.__serializedModels then
		url ..= "?isolateModels=true"
	end
	local request_body = Http.msgpackEncode({ sessionId = self.__sessionId, ids = ids })

	return Http.post(url, request_body)
		:andThen(rejectFailedRequests)
		:andThen(Http.Response.msgpack)
		:andThen(function(response_body)
			if response_body.sessionId ~= self.__sessionId then
				return Promise.reject("Server changed ID")
			end

			if not self.__serializedModels then
				assert(validateApiSerialize(response_body))
			end

			return response_body
		end)
end

function ApiContext:output(entries)
	local url = ("%s/api/output"):format(self.__baseUrl)
	local body = Http.msgpackEncode({ sessionId = self.__sessionId, entries = entries })
	return Http.post(url, body):andThen(rejectFailedRequests):andThen(Http.Response.msgpack):andThen(function(response)
		if type(response) ~= "table" or response.sessionId ~= self.__sessionId then
			return Promise.reject("Server changed ID while forwarding Studio output")
		end
		return response
	end)
end

-- Private opt-in relay. No public headless handle exposes debugger consent.
function ApiContext:__debuggerRelay(body, relayId)
	if not self.__connected then
		return Promise.reject("Disconnected debugger relay")
	end
	local sessionId = self.__sessionId
	local url = ("%s/api/debugger/relay"):format(self.__baseUrl)
	local request
	if body then
		body.sessionId = sessionId
		request = Http.post(url, Http.msgpackEncode(body))
	else
		request = Http.get(url .. "?sessionId=" .. sessionId .. "&relayId=" .. relayId)
	end
	self.__activeRequests[request] = true
	return request
		:andThen(rejectFailedRequests)
		:andThen(Http.Response.msgpack)
		:andThen(function(response)
			if not self.__connected or self.__sessionId ~= sessionId or response.sessionId ~= sessionId then
				return Promise.reject("Debugger session changed")
			end
			return response
		end)
		:finally(function()
			self.__activeRequests[request] = nil
		end)
end

function ApiContext:refPatch(ids: { string })
	local url = ("%s/api/ref-patch"):format(self.__baseUrl)
	local request_body = Http.msgpackEncode({ sessionId = self.__sessionId, ids = ids })

	return Http.post(url, request_body)
		:andThen(rejectFailedRequests)
		:andThen(Http.Response.msgpack)
		:andThen(function(response_body)
			if response_body.sessionId ~= self.__sessionId then
				return Promise.reject("Server changed ID")
			end

			assert(validateApiRefPatch(response_body))

			return response_body
		end)
end

return ApiContext
