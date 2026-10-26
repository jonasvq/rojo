local HttpService = game:GetService("HttpService")
local Config = require(script.Parent.Config)
local Settings = require(script.Parent.Settings)
local ApiContext = require(script.Parent.ApiContext)
local Util = require(script.Parent.HeadlessUtil)
local Log = require(script.Parent.Parent.Packages.Log)

local Headless = {}
Headless.__index = Headless
local discoveries = {}
local names = {
	Connected = true,
	Address = true,
	ProjectName = true,
	Changed = true,
	ConnectAsync = true,
	DisconnectAsync = true,
	GetSetting = true,
	SetSetting = true,
	Notify = true,
	GetHostAndPort = true,
	CreateApiContext = true,
}
local fields = { Connected = true, Address = true, ProjectName = true }

-- The generated module links to this exact runtime, including when another
-- Studio plugin requires it. No caller is inferred from ambient global state.
function Headless.resolve(key)
	return discoveries[key]
end

local function validText(text, limit)
	assert(type(text) == "string" and #text <= limit and utf8.len(text), "Invalid API text")
	return text
end

local function protect(record, callback, ...)
	if record.alive then
		local ok, err = pcall(callback, ...)
		if not ok then
			Log.warn("Third-party plugin callback failed: {}", err)
		end
	end
end

function Headless.new(options)
	local self = setmetatable({
		bridge = options.bridge,
		notify = options.notify,
		onStateChanged = options.onStateChanged or function() end,
		root = options.root or game,
		records = {},
		dead = setmetatable({}, { __mode = "k" }),
		queue = {},
		enabled = false,
		alive = true,
		key = HttpService:GenerateGUID(false),
		connections = {},
		permissionTimeout = 60,
		childNameConnections = {},
	}, Headless)
	self.api = table.freeze({
		Version = table.freeze(Util.copy(Config.version)),
		ProtocolVersion = Config.protocolVersion,
		RequestAccess = function(_, caller, requested)
			return self:requestAccess(caller, requested)
		end,
	})
	discoveries[self.key] = self.api
	self.last = self.bridge:snapshot()
	self.cleanupBridge = self.bridge:observe(function(snapshot)
		local old = self.last
		self.last = snapshot
		for _, record in self.records do
			for field in fields do
				if
					record.alive
					and record.grants.Changed
					and record.grants[field]
					and snapshot[field] ~= old[field]
				then
					for _, callback in record.listeners do
						task.spawn(protect, record, callback, field, snapshot[field], old[field])
					end
				end
			end
		end
	end)
	local function refresh()
		task.defer(function()
			if self.alive then
				self:publish()
			end
		end)
	end
	local function watchName(child)
		self.childNameConnections[child] = child:GetPropertyChangedSignal("Name"):Connect(refresh)
	end
	for _, child in self.root:GetChildren() do
		watchName(child)
	end
	table.insert(
		self.connections,
		self.root.ChildAdded:Connect(function(child)
			watchName(child)
			refresh()
		end)
	)
	table.insert(
		self.connections,
		self.root.ChildRemoved:Connect(function(child)
			if self.childNameConnections[child] then
				self.childNameConnections[child]:Disconnect()
				self.childNameConnections[child] = nil
			end
			refresh()
		end)
	)
	self.cleanupSetting = Settings:onChanged("enableHeadlessAPI", function(value)
		if self.alive then
			self:setEnabled(value == true)
		end
	end)
	self:setEnabled(Settings:get("enableHeadlessAPI") == true)
	return self
end

function Headless:state()
	local grants = {}
	for _, record in self.records do
		if record.alive then
			local granted = {}
			for name, allowed in record.grants do
				if allowed then
					table.insert(granted, name)
				end
			end
			table.sort(granted)
			if #granted > 0 then
				table.insert(grants, {
					label = record.label,
					names = granted,
					revoke = function()
						self:revoke(record)
					end,
				})
			end
		end
	end
	return {
		prompt = self.active and self.active.prompt,
		grants = grants,
		available = self.owned ~= nil,
		status = if not self.enabled
			then "Plugin API is disabled"
			elseif self.projectOwnsNamespace then "Project manages game.Rojo"
			elseif not self.owned then "Discovery unavailable: game.Rojo is occupied"
			else "Discovery available",
	}
end

function Headless:changed()
	self.onStateChanged(self:state())
end

function Headless:unpublish()
	local owned = self.owned
	self.owned = nil
	if owned then
		owned:Destroy()
	end
end

function Headless:publish()
	if not self.alive or not self.enabled or self.projectOwnsNamespace then
		self:unpublish()
		return
	end
	local existing = self.root:FindFirstChild("Rojo")
	for _, child in self.root:GetChildren() do
		if child.Name == "Rojo" and child ~= self.owned then
			existing = child
			break
		end
	end
	if self.owned and (self.owned.Parent ~= self.root or self.owned.Name ~= "Rojo" or existing ~= self.owned) then
		self:unpublish()
	end
	if not self.owned and not self.root:FindFirstChild("Rojo") then
		local module = Instance.new("ModuleScript")
		module.Name = "Rojo"
		module.Archivable = false
		module.Source = ("return require(script.Dispatch.Value).resolve(%q)"):format(self.key)
		local dispatch = Instance.new("ObjectValue")
		dispatch.Name = "Dispatch"
		dispatch.Value = script
		dispatch.Parent = module
		self.owned = module
		module.Parent = self.root
	end
	self:changed()
end

function Headless:beforeHydrate(info, instances)
	local root = instances[info.rootInstanceId]
	self.managedRojoIds = {}
	if root and root.ClassName == "DataModel" then
		for _, id in root.Children do
			if instances[id] and instances[id].Name == "Rojo" then
				self.managedRojoIds[id] = true
			end
		end
	end
	self.projectOwnsNamespace = next(self.managedRojoIds) ~= nil
	if self.projectOwnsNamespace then
		self:unpublish()
	else
		self:publish()
	end
end

function Headless:beforePatch(patch, instanceMap)
	local rootId = instanceMap.fromInstances[game]
	if not rootId then
		return
	end
	self.managedRojoIds = self.managedRojoIds or {}
	for _, id in patch.removed do
		self.managedRojoIds[id] = nil
	end
	for id, instance in patch.added do
		if instance.Parent == rootId and instance.Name == "Rojo" then
			self.managedRojoIds[id] = true
		end
	end
	for _, update in patch.updated do
		local instance = instanceMap.fromIds[update.id]
		if update.changedName ~= nil and instance and instance.Parent == self.root then
			self.managedRojoIds[update.id] = if update.changedName == "Rojo" then true else nil
		end
	end
	self.projectOwnsNamespace = next(self.managedRojoIds) ~= nil
	if self.projectOwnsNamespace then
		self:unpublish()
	else
		self:publish()
	end
end

function Headless:setEnabled(enabled)
	self.enabled = enabled
	if not enabled then
		self:unpublish()
		for _, record in self.records do
			self:revoke(record)
		end
		if self.active then
			self:finish(self.active, false)
		end
		while #self.queue > 0 do
			self:finish(table.remove(self.queue, 1), false)
		end
	else
		self:publish()
	end
	self:changed()
end

function Headless:check(record, name)
	assert(self.alive and self.enabled and record.alive, "Rojo API access was revoked")
	assert(record.grants[name], "Rojo API access denied: " .. name)
end

function Headless:track(record, promise)
	record.requests[promise] = true
	local ok, value = promise:await()
	record.requests[promise] = nil
	assert(record.alive and self.alive and self.enabled, "Rojo API access was revoked")
	assert(ok, tostring(value))
	return value
end

function Headless:revoke(record)
	if not record.alive then
		return
	end
	record.alive = false
	for request in record.requests do
		request:cancel()
	end
	table.clear(record.requests)
	for close in record.contexts do
		close()
	end
	table.clear(record.contexts)
	for dismiss in record.notifications do
		dismiss()
	end
	table.clear(record.notifications)
	for _, connection in record.connections do
		connection:Disconnect()
	end
	table.clear(record.connections)
	table.clear(record.listeners)
	for cancel in record.waiters do
		cancel()
	end
	if self.active and self.active.record == record then
		self:finish(self.active, false)
	end
	for i = #self.queue, 1, -1 do
		if self.queue[i].record == record then
			self:finish(table.remove(self.queue, i), false)
		end
	end
	for caller, current in self.records do
		if current == record then
			self.records[caller] = nil
			break
		end
	end
	self:changed()
end

function Headless:record(caller)
	local record = self.records[caller]
	if record and record.alive then
		return record
	end
	local label = caller.Name:gsub("[%c]", " ")
	if not utf8.len(label) then
		label = "[invalid plugin name]"
	elseif #label > 100 then
		label = label:sub(1, utf8.offset(label, 0, 101) - 1)
	end
	record = {
		alive = true,
		label = label,
		grants = {},
		requests = {},
		contexts = {},
		notifications = {},
		listeners = {},
		connections = {},
		waiters = {},
	}
	self.records[caller] = record
	local function unload()
		self.dead[caller] = true
		self:revoke(record)
	end
	table.insert(record.connections, caller.Unloading:Connect(unload))
	table.insert(record.connections, caller.Destroying:Connect(unload))
	record.handle = self:handle(record)
	return record
end

function Headless:requestAccess(caller, requested)
	assert(self.alive and self.enabled, "Rojo API is unavailable")
	assert(
		typeof(caller) == "Instance" and caller:IsA("Plugin") and not self.dead[caller],
		"Expected a live Plugin Instance"
	)
	-- Destroyed Plugin objects still answer native queries and accept new signal
	-- connections. Require attached plugin code before starting lifetime tracking;
	-- names, Parent == nil and IsActivated are not evidence that it is loaded.
	assert(caller:FindFirstChildWhichIsA("LuaSourceContainer", true), "Expected a Plugin with attached code")
	assert(type(requested) == "table" and #requested >= 1 and #requested <= 11, "Expected 1..11 API names")
	local unique = {}
	for key, name in requested do
		assert(
			type(key) == "number" and key % 1 == 0 and key >= 1 and key <= #requested and names[name],
			"Unknown API name or invalid name list"
		)
		unique[name] = true
	end
	for i = 1, #requested do
		assert(names[requested[i]], "Sparse API name list")
	end
	local record = self:record(caller)
	local missing = {}
	for name in unique do
		if not record.grants[name] then
			table.insert(missing, name)
		end
	end
	table.sort(missing)
	if #missing > 0 then
		assert(#self.queue + (if self.active then 1 else 0) < 8, "Rojo API permission queue is full")
		local request = { record = record, names = missing, done = false, event = Instance.new("BindableEvent") }
		table.insert(self.queue, request)
		self:pump()
		if not request.done then
			request.event.Event:Wait()
		end
		request.event:Destroy()
	end
	local granted = {}
	for name in unique do
		granted[name] = record.alive and record.grants[name] == true
	end
	return table.freeze(granted), record.handle
end

function Headless:pump()
	if self.active or not self.alive or not self.enabled or #self.queue == 0 then
		return
	end
	local request = table.remove(self.queue, 1)
	if not request.record.alive then
		self:finish(request, false)
		return
	end
	self.active = request
	local undecided = {}
	for _, name in request.names do
		if not request.record.grants[name] then
			undecided[name] = false
		end
	end
	request.decisions = {}
	for _, name in request.names do
		if request.record.grants[name] then
			request.decisions[name] = true
		end
	end
	request.prompt = {
		label = request.record.label,
		names = request.names,
		decisions = request.decisions,
		decide = function(name, allowed)
			if self.active ~= request or not table.find(request.names, name) or type(allowed) ~= "boolean" then
				return
			end
			request.decisions[name] = allowed
			for _, pending in request.names do
				if request.decisions[pending] == nil then
					self:changed()
					return
				end
			end
			self:finish(request, true)
		end,
		deny = function()
			self:finish(request, false)
		end,
	}
	request.timeout = task.delay(self.permissionTimeout, function()
		self:finish(request, false)
	end)
	self:changed()
	if next(undecided) == nil then
		self:finish(request, true)
	end
end

function Headless:finish(request, decided)
	if request.done then
		return
	end
	request.done = true
	if request.timeout and request.timeout ~= coroutine.running() then
		task.cancel(request.timeout)
	end
	if decided and self.enabled and request.record.alive then
		for _, name in request.names do
			if request.decisions[name] then
				request.record.grants[name] = true
			end
		end
	end
	if self.active == request then
		self.active = nil
	end
	request.event:Fire()
	self:changed()
	task.defer(function()
		self:pump()
	end)
end

function Headless:event(record)
	return table.freeze({
		Connect = function(_, callback)
			self:check(record, "Changed")
			assert(type(callback) == "function", "Expected callback")
			local key = {}
			record.listeners[key] = callback
			local connected = true
			return table.freeze({
				Disconnect = function()
					connected = false
					record.listeners[key] = nil
				end,
				IsConnected = function()
					return connected and record.alive
				end,
			})
		end,
		Wait = function()
			self:check(record, "Changed")
			local event = Instance.new("BindableEvent")
			local cancelled = false
			local function cancel()
				cancelled = true
				event:Fire()
			end
			record.waiters[cancel] = true
			local function callback(...)
				event:Fire(...)
			end
			local key = {}
			record.listeners[key] = callback
			local result = table.pack(event.Event:Wait())
			record.listeners[key] = nil
			record.waiters[cancel] = nil
			event:Destroy()
			assert(not cancelled and record.alive, "Rojo API access was revoked")
			return table.unpack(result, 1, result.n)
		end,
	})
end

function Headless:setting(record, name, value, write)
	self:check(record, if write then "SetSetting" else "GetSetting")
	assert(type(name) == "string" and name ~= "priorEndpoints", "Preference is not exposed through the API")
	local current = Settings:get(name)
	if not write then
		return Util.copy(current)
	end
	assert(type(value) == type(Settings:__default(name)), "Preference type mismatch")
	if name == "twoWaySync" then
		assert(self.bridge.app.serveSession == nil, "Disconnect before changing twoWaySync")
	end
	if name == "largeChangesConfirmationThreshold" then
		assert(value % 1 == 0 and value >= 1 and value <= 999, "Threshold must be 1..999")
	end
	if name == "confirmationBehavior" then
		assert(
			table.find({ "Initial", "Always", "Large Changes", "Unlisted PlaceId", "Never" }, value),
			"Invalid confirmation behavior"
		)
	end
	if name == "syncReminderMode" then
		assert(table.find({ "None", "Notify", "Fullscreen" }, value), "Invalid reminder mode")
	end
	if name == "logLevel" then
		assert(Log.Level[value], "Invalid log level")
	end
	Settings:set(name, Util.copy(value))
end

function Headless:context(record, url)
	self:check(record, "CreateApiContext")
	local api = ApiContext.new(Util.baseUrl(url))
	local alive = true
	local requests = {}
	local function close()
		if not alive then
			return
		end
		alive = false
		for request in requests do
			request:cancel()
		end
		table.clear(requests)
		api:disconnect()
	end
	record.contexts[close] = true
	local facade = {}
	for _, name in
		{
			"connect",
			"read",
			"write",
			"serialize",
			"refPatch",
			"open",
			"setMessageCursor",
			"connectWebSocket",
			"disconnect",
		}
	do
		facade[name] = function(_, ...)
			self:check(record, "CreateApiContext")
			assert(alive, "API context is closed")
			if name == "disconnect" then
				close()
				record.contexts[close] = nil
				return
			end
			local args = table.pack(...)
			if name == "connectWebSocket" then
				assert(type(args[1]) == "table", "Expected socket packet handlers")
				local handlers = {}
				for kind, callback in args[1] do
					assert(type(kind) == "string" and type(callback) == "function", "Invalid socket packet handler")
					handlers[kind] = function(...)
						if alive and self.alive and self.enabled then
							protect(record, callback, ...)
						end
					end
				end
				args[1] = handlers
			end
			local result = api[name](api, table.unpack(args, 1, args.n))
			if name == "setMessageCursor" then
				return result
			end
			requests[result] = true
			result:finally(function()
				requests[result] = nil
			end)
			return result
		end
	end
	return table.freeze(facade)
end

function Headless:notification(record, options)
	self:check(record, "Notify")
	assert(type(options) == "table", "Expected notification options")
	local timeout = options.timeout or 5
	assert(type(timeout) == "number" and timeout >= 1 and timeout <= 60, "Notification timeout must be 1..60")
	local count = 0
	for _ in record.notifications do
		count += 1
	end
	assert(count < 32, "Too many active plugin notifications")
	local actions = {}
	local entries = options.actions or {}
	assert(type(entries) == "table" and #entries <= 3, "Expected at most three actions")
	for i, action in entries do
		assert(
			type(i) == "number"
				and i % 1 == 0
				and i >= 1
				and i <= #entries
				and type(action) == "table"
				and type(action.onClick) == "function",
			"Invalid notification action"
		)
		actions[tostring(i)] = {
			text = validText(action.text, 80),
			style = "Bordered",
			layoutOrder = i,
			onClick = function()
				protect(record, action.onClick)
			end,
		}
	end
	local dismiss
	local closed = false
	local function cleanup()
		if closed then
			return
		end
		closed = true
		if dismiss then
			record.notifications[dismiss] = nil
		end
	end
	local closeUi = self.notify({
		text = ("[Plugin %s] %s"):format(record.label, validText(options.text, 2048)),
		timeout = timeout,
		actions = actions,
		onClose = cleanup,
	})
	dismiss = function()
		if not closed then
			cleanup()
			if closeUi then
				closeUi()
			end
		end
	end
	if not closed and closeUi then
		record.notifications[dismiss] = true
		task.delay(timeout, dismiss)
	end
	return dismiss
end

function Headless:handle(record)
	local event = self:event(record)
	local methods = {
		ConnectAsync = function(_, host, port)
			self:check(record, "ConnectAsync")
			return self:track(record, self.bridge:connect(host, port))
		end,
		DisconnectAsync = function()
			self:check(record, "DisconnectAsync")
			return self.bridge:disconnect()
		end,
		GetHostAndPort = function()
			self:check(record, "GetHostAndPort")
			return self.bridge.app:getHostAndPort()
		end,
		GetSetting = function(_, name)
			return self:setting(record, name, nil, false)
		end,
		SetSetting = function(_, name, value)
			return self:setting(record, name, value, true)
		end,
		Notify = function(_, options)
			return self:notification(record, options)
		end,
		CreateApiContext = function(_, url)
			return self:context(record, url)
		end,
	}
	return table.freeze(setmetatable(
		{},
		table.freeze({
			__index = function(_, name)
				self:check(record, name)
				if fields[name] then
					return self.bridge:snapshot()[name]
				end
				if name == "Changed" then
					return event
				end
				return methods[name]
			end,
			__newindex = function()
				error("Rojo client is read-only", 2)
			end,
		})
	))
end

function Headless:destroy()
	if not self.alive then
		return
	end
	self.alive = false
	self:setEnabled(false)
	self.cleanupBridge()
	self.cleanupSetting()
	for _, connection in self.connections do
		connection:Disconnect()
	end
	for _, connection in self.childNameConnections do
		connection:Disconnect()
	end
	table.clear(self.childNameConnections)
	discoveries[self.key] = nil
	self:unpublish()
end

return Headless
