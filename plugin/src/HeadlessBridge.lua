local Promise = require(script.Parent.Parent.Packages.Promise)
local ServeSession = require(script.Parent.ServeSession)
local Util = require(script.Parent.HeadlessUtil)
local Log = require(script.Parent.Parent.Packages.Log)

local Bridge = {}
Bridge.__index = Bridge

function Bridge.new(app)
	local self = setmetatable({ app = app, listeners = {}, alive = true }, Bridge)
	self.last = self:snapshot()
	return self
end

function Bridge:snapshot()
	local app = self.app
	local host, port = app:getHostAndPort()
	local session = app.serveSession
	return table.freeze({
		Connected = session ~= nil
			and session:getStatus() == ServeSession.Status.Connected
			and session.__apiContext.__socketOpen == true,
		Address = host .. ":" .. tostring(port),
		ProjectName = if session and session.__serverInfo then session.__serverInfo.projectName else nil,
	})
end

function Bridge:observe(callback)
	self.listeners[callback] = true
	return function()
		self.listeners[callback] = nil
	end
end

function Bridge:refresh(detail)
	local snapshot = self:snapshot()
	self.last = snapshot
	for callback in self.listeners do
		local ok, err = pcall(callback, snapshot, detail)
		if not ok then
			Log.warn("Headless observer failed: {}", err)
		end
	end
end

function Bridge:bindSession(session)
	if self.sessionCleanup then
		self.sessionCleanup()
	end
	self.sessionCleanup = session:observeReady(function(_, _, detail)
		self:refresh(detail)
	end)
end

function Bridge:connect(host, port)
	assert(self.alive, "Rojo unloaded")
	local selectedHost, selectedPort = self.app:getHostAndPort()
	host, port = Util.hostPort(host or selectedHost, port or selectedPort)
	local app = self.app
	if app.serveSession then
		local activeHost, activePort = Util.hostPort(selectedHost, selectedPort)
		assert(
			Util.endpoint(host, port) == Util.endpoint(activeHost, activePort),
			"A different Rojo connection is already active"
		)
	end
	return Promise.new(function(resolve, reject, onCancel)
		local cleanup
		local function finish(ok, value)
			if cleanup then
				cleanup()
				cleanup = nil
			end
			if ok then
				resolve(value)
			else
				reject(value)
			end
		end
		cleanup = self:observe(function(snapshot, detail)
			if not self.alive then
				finish(false, "Rojo unloaded")
			elseif snapshot.Connected then
				finish(true, snapshot)
			elseif not self.alive or not app.serveSession then
				finish(false, detail or "Connection cancelled or failed")
			end
		end)
		onCancel(function()
			if cleanup then
				cleanup()
				cleanup = nil
			end
		end)
		if not app.serveSession then
			app.setHost(host)
			app.setPort(port)
			local ok, err = pcall(app.startSession, app)
			if not ok then
				finish(false, err)
				return
			end
		end
		local snapshot = self:snapshot()
		if snapshot.Connected then
			finish(true, snapshot)
		elseif not app.serveSession then
			finish(false, (app.state and app.state.errorMessage) or "Connection refused")
		end
	end)
end

function Bridge:disconnect()
	assert(self.alive, "Rojo unloaded")
	self.app:endSession()
	self:refresh()
	return self:snapshot()
end

function Bridge:destroy()
	self.alive = false
	if self.sessionCleanup then
		self.sessionCleanup()
		self.sessionCleanup = nil
	end
	self:refresh("Rojo unloaded")
	table.clear(self.listeners)
end

return Bridge
