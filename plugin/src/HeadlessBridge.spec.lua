return function()
	local Bridge = require(script.Parent.HeadlessBridge)
	local ServeSession = require(script.Parent.ServeSession)
	local Util = require(script.Parent.HeadlessUtil)
	local function fixture()
		local host, port = "localhost", "34872"
		local calls = 0
		local app = {
			state = {},
			getHostAndPort = function()
				return host, port
			end,
			setHost = function(value)
				host = value
			end,
			setPort = function(value)
				port = value
			end,
		}
		local bridge = Bridge.new(app)
		function app:startSession()
			calls += 1
			self.serveSession = {
				__apiContext = { __socketOpen = false },
				__serverInfo = { projectName = "Project" },
				getStatus = function()
					return ServeSession.Status.Connecting
				end,
			}
			bridge:refresh()
		end
		function app:endSession()
			self.serveSession = nil
			bridge:refresh()
		end
		return bridge, app, function()
			return calls
		end
	end
	it("waits for initial synchronization and socket readiness while joining matching requests", function()
		local bridge, app, calls = fixture()
		local first = bridge:connect()
		local second = bridge:connect("http://LOCALHOST", 34872)
		local completed = 0
		first:andThen(function(snapshot)
			assert(snapshot.Connected and table.isfrozen(snapshot), "Ready result must be immutable")
			completed += 1
		end)
		second:andThen(function()
			completed += 1
		end)
		expect(calls()).to.equal(1)
		expect(pcall(bridge.connect, bridge, "different", 34872)).to.equal(false)
		app.serveSession.getStatus = function()
			return ServeSession.Status.Connected
		end
		bridge:refresh()
		task.wait()
		expect(completed).to.equal(0)
		app.serveSession.__apiContext.__socketOpen = true
		bridge:refresh()
		task.wait()
		expect(completed).to.equal(2)
		expect(bridge:connect():await()).to.equal(true)
		bridge:disconnect()
		expect(app.serveSession).to.equal(nil)
		bridge:disconnect()
		bridge:destroy()
	end)

	it("rejects lock denial, cancelled or failed startup and unload without retaining waiters", function()
		local bridge, app = fixture()
		app.startSession = function()
			app.state.errorMessage = "Sync lock held"
		end
		expect(bridge:connect():await()).to.equal(false)
		expect(next(bridge.listeners)).to.equal(nil)
		bridge:destroy()
		bridge, app = fixture()
		app.startSession = function()
			error("Startup exception", 0)
		end
		expect(bridge:connect():await()).to.equal(false)
		expect(next(bridge.listeners)).to.equal(nil)
		bridge:destroy()
		bridge, app = fixture()
		local pending = bridge:connect()
		app:endSession()
		expect(pending:await()).to.equal(false)
		expect(next(bridge.listeners)).to.equal(nil)
		local cancelled = bridge:connect()
		cancelled:cancel()
		expect(next(bridge.listeners)).to.equal(nil)
		local unloaded = bridge:connect()
		bridge:destroy()
		expect(unloaded:await()).to.equal(false)
	end)

	it("validates endpoints before changing bindings and rejects unsafe context URLs", function()
		local bridge, app, calls = fixture()
		for _, host in { "user@host", "host/path", "http://host/path", "host:9000", "" } do
			expect(pcall(bridge.connect, bridge, host, 34872)).to.equal(false)
		end
		for _, port in { 0, -1, 1.5, 65536, "no port" } do
			expect(pcall(bridge.connect, bridge, "host", port)).to.equal(false)
		end
		expect(calls()).to.equal(0)
		local host, port = app:getHostAndPort()
		expect(host).to.equal("localhost")
		expect(port).to.equal("34872")
		expect(Util.baseUrl("https://example.test")).to.equal("https://example.test:443")
		expect(Util.baseUrl("http://[::1]:9000/")).to.equal("http://[::1]:9000")
		bridge:destroy()
	end)

	it("reports ready state without replacing the existing UI status callback", function()
		local observed
		local api = {
			__socketOpen = false,
			disconnect = function() end,
			__observeSocket = function(_, callback)
				observed = callback
				return function()
					observed = nil
				end
			end,
		}
		local session = ServeSession.new({ apiContext = api, twoWaySync = false })
		local uiCalls, ready = 0, {}
		session:onStatusChanged(function()
			uiCalls += 1
		end)
		local cleanup = session:observeReady(function(value)
			table.insert(ready, value)
		end)
		session:__setStatus(ServeSession.Status.Connected)
		expect(ready[#ready]).to.equal(false)
		api.__socketOpen = true
		observed()
		expect(ready[#ready]).to.equal(true)
		session:stop()
		expect(uiCalls).to.equal(2)
		expect(ready[#ready]).to.equal(false)
		expect(observed).to.equal(nil)
		cleanup()
	end)
end
