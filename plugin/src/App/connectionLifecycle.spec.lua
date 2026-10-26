return function()
	local Plugin = script.Parent.Parent
	local Promise = require(Plugin.Parent.Packages.Promise)
	local App = require(script.Parent)({}).component
	local ApiContext = require(Plugin.ApiContext)
	local ServeSession = require(Plugin.ServeSession)

	it("does not restart a stopped session", function()
		local calls = 0
		local api = {}
		function api:connect()
			calls += 1
			return Promise.new(function() end)
		end
		function api:disconnect() end
		local session = ServeSession.new({ apiContext = api, twoWaySync = false })
		session:start()
		session:stop()
		session:start()
		expect(calls).to.equal(1)
		expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
	end)

	it("clears an immediately failed session so the next Connect can retry", function()
		local original = ServeSession.new
		local created = 0
		ServeSession.new = function()
			created += 1
			local session = {}
			function session:setUpdateLoadingTextCallback() end
			function session:hookPrecommit()
				return function() end
			end
			function session:hookPostcommit()
				return function() end
			end
			function session:setConfirmCallback() end
			function session:onStatusChanged(callback)
				self.callback = callback
			end
			function session:start()
				self.callback(ServeSession.Status.Disconnected, "startup failed")
			end
			return session
		end
		local app = {
			claimSyncLock = function()
				return true
			end,
			releaseSyncLock = function() end,
			clearRunningConnectionInfo = function() end,
			getHostAndPort = function()
				return "localhost", "1"
			end,
			setState = function() end,
			addNotification = function() end,
		}
		local ok, err = pcall(function()
			App.startSession(app)
			expect(app.serveSession).to.equal(nil)
			App.startSession(app)
			expect(created).to.equal(2)
			expect(app.serveSession).to.equal(nil)
		end)
		ServeSession.new = original
		assert(ok, tostring(err))
	end)

	it("starts only one API connection when the session is started twice", function()
		local calls = 0
		local api = {}
		function api:connect()
			calls += 1
			return Promise.new(function() end)
		end
		function api:disconnect() end
		local session = ServeSession.new({ apiContext = api, twoWaySync = false })
		local ok, err = pcall(function()
			session:start()
			session:start()
			expect(calls).to.equal(1)
		end)
		session:stop()
		assert(ok, tostring(err))
	end)

	it("keeps one session and sync lock claim for rapid Connect actions", function()
		local originalApiNew = ApiContext.new
		local originalSessionNew = ServeSession.new
		local sessions = {}
		local claims, connects = 0, 0
		ApiContext.new = function()
			local api = {}
			function api:connect()
				connects += 1
				return Promise.new(function() end)
			end
			function api:disconnect() end
			return api
		end
		ServeSession.new = function(options)
			local session = originalSessionNew(options)
			table.insert(sessions, session)
			return session
		end
		local app = {
			claimSyncLock = function()
				claims += 1
				return true
			end,
			releaseSyncLock = function() end,
			clearRunningConnectionInfo = function() end,
			getHostAndPort = function()
				return "localhost", "1"
			end,
			setState = function() end,
			addNotification = function() end,
		}
		local ok, err = pcall(function()
			App.startSession(app)
			local first = app.serveSession
			App.startSession(app)
			expect(#sessions).to.equal(1)
			expect(claims).to.equal(1)
			expect(connects).to.equal(1)
			expect(app.serveSession).to.equal(first)
		end)
		ApiContext.new = originalApiNew
		ServeSession.new = originalSessionNew
		for _, session in sessions do
			session:stop()
		end
		assert(ok, tostring(err))
	end)
end
