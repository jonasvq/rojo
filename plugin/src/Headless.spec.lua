return function()
	local Headless = require(script.Parent.Headless)
	local Promise = require(script.Parent.Parent.Packages.Promise)
	local Settings = require(script.Parent.Settings)
	local fixtures = {}
	local function fixture()
		local root = Instance.new("Folder")
		local state = { Connected = false, Address = "localhost:34872", ProjectName = nil }
		local listeners = {}
		local bridge = { app = {
			getHostAndPort = function()
				return "localhost", "34872"
			end,
		} }
		function bridge:snapshot()
			return table.freeze(table.clone(state))
		end
		function bridge:observe(callback)
			listeners[callback] = true
			return function()
				listeners[callback] = nil
			end
		end
		function bridge:emit(values)
			for key, value in values do
				state[key] = value
			end
			for callback in listeners do
				callback(self:snapshot())
			end
		end
		function bridge:connect()
			return Promise.resolve(self:snapshot())
		end
		function bridge:disconnect()
			state.Connected = false
			return self:snapshot()
		end
		local notifications = {}
		local controller = Headless.new({
			root = root,
			bridge = bridge,
			notify = function(options)
				table.insert(notifications, options)
				return function()
					options.dismissed = true
				end
			end,
		})
		local callers = {}
		local function caller(name)
			-- PluginManager is native Studio's legacy global, verified by the probe.
			-- selene: allow(undefined_variable)
			local pluginObject = PluginManager():CreatePlugin()
			pluginObject.Name = name or "Companion"
			local code = Instance.new("ModuleScript")
			code.Name = "CompanionCode"
			code.Parent = pluginObject
			table.insert(callers, pluginObject)
			return pluginObject
		end
		table.insert(fixtures, function()
			controller:destroy()
			for _, item in callers do
				item:Destroy()
			end
			root:Destroy()
		end)
		return controller, caller, bridge, root, notifications
	end
	afterEach(function()
		for _, cleanup in fixtures do
			cleanup()
		end
		table.clear(fixtures)
	end)
	local function request(controller, caller, names)
		local result = {}
		task.spawn(function()
			result.ok, result.grants, result.client = pcall(controller.api.RequestAccess, controller.api, caller, names)
			result.done = true
		end)
		return result
	end
	local function grant(controller, caller, names)
		local result = request(controller, caller, names)
		local active = controller.active
		if active then
			for _, name in table.clone(active.names) do
				active.prompt.decide(name, true)
			end
		end
		task.wait()
		assert(result.done and result.ok, tostring(result.grants))
		return result.client, result.grants
	end

	it("publishes immutable native discovery, preserves collisions and unpublishes before root hydration", function()
		local controller, caller, _, root = fixture()
		local module = root:FindFirstChild("Rojo")
		expect(module ~= nil and module:IsA("ModuleScript") and not module.Archivable).to.equal(true)
		expect(require(module)).to.equal(controller.api)
		expect(table.isfrozen(controller.api) and table.isfrozen(controller.api.Version)).to.equal(true)
		expect(pcall(function()
			controller.api.ProtocolVersion = 99
		end)).to.equal(false)
		local client = grant(controller, caller(), { "Connected" })
		controller:beforeHydrate(
			{ rootInstanceId = "ROOT" },
			{ ROOT = { ClassName = "DataModel", Children = { "MANAGED" } }, MANAGED = { Name = "Rojo" } }
		)
		expect(root:FindFirstChild("Rojo")).to.equal(nil)
		expect(client.Connected).to.equal(false)
		local foreign = Instance.new("Folder")
		foreign.Name = "Rojo"
		foreign.Parent = root
		controller.projectOwnsNamespace = false
		controller:publish()
		expect(root:FindFirstChild("Rojo")).to.equal(foreign)
		expect(controller.owned).to.equal(nil)
		foreign.Name = "Other"
		task.wait()
		expect(controller.owned ~= nil).to.equal(true)
		controller:destroy()
		expect(foreign.Parent).to.equal(root)
	end)

	it("releases discovery for live project ownership and republishes after managed removal", function()
		local controller = fixture()
		local map = { fromInstances = { [game] = "ROOT" }, fromIds = {} }
		controller:beforePatch(
			{ added = { MANAGED = { Name = "Rojo", Parent = "ROOT" } }, removed = {}, updated = {} },
			map
		)
		expect(controller.owned).to.equal(nil)
		controller:beforePatch({ added = {}, removed = { "MANAGED" }, updated = {} }, map)
		expect(controller.owned ~= nil).to.equal(true)
	end)

	it("isolates mixed grants by actual caller, rejects mutation and reprompts only additional names", function()
		local controller, caller = fixture()
		local first, second = caller("same name"), caller("same name")
		local result = request(controller, first, { "Connected", "GetHostAndPort", "Connected" })
		expect(result.done).to.equal(nil)
		controller.active.prompt.decide("Connected", true)
		controller.active.prompt.decide("GetHostAndPort", false)
		task.wait()
		expect(result.grants.Connected).to.equal(true)
		expect(result.grants.GetHostAndPort).to.equal(false)
		expect(table.isfrozen(result.grants)).to.equal(true)
		expect(pcall(function()
			return result.client:GetHostAndPort()
		end)).to.equal(false)
		expect(pcall(function()
			result.client.Connected = true
		end)).to.equal(false)
		local repeated = request(controller, first, { "Connected" })
		expect(repeated.done).to.equal(true)
		local extra = request(controller, first, { "Connected", "Address" })
		expect(#controller.active.names).to.equal(1)
		expect(controller.active.names[1]).to.equal("Address")
		controller.active.prompt.decide("Address", true)
		task.wait()
		expect(extra.grants.Address).to.equal(true)
		local other = request(controller, second, { "Connected" })
		expect(other.done).to.equal(nil)
		controller.active.prompt.deny()
		task.wait()
		expect(other.grants.Connected).to.equal(false)
		expect(pcall(function()
			return other.client.Connected
		end)).to.equal(false)
	end)

	it("bounds and serializes permission requests, denying cancellation and caller unload", function()
		local controller, caller = fixture()
		local pluginObject = caller()
		for _, invalid in { { "Unknown" }, { [1] = "Connected", [3] = "Address" }, {}, { Connected = true } } do
			expect(pcall(controller.api.RequestAccess, controller.api, pluginObject, invalid)).to.equal(false)
		end
		local folder = Instance.new("Folder")
		expect(pcall(controller.api.RequestAccess, controller.api, folder, { "Connected" })).to.equal(false)
		folder:Destroy()
		local pending = {}
		for _ = 1, 8 do
			table.insert(pending, request(controller, caller(), { "Connected" }))
		end
		expect(#controller.queue).to.equal(7)
		local overflow = request(controller, caller(), { "Connected" })
		expect(overflow.ok).to.equal(false)
		local active = controller.active
		-- This invokes the same deny path used by the sixty-second timeout.
		controller:finish(active, false)
		task.wait()
		expect(pending[1].grants.Connected).to.equal(false)
		controller:setEnabled(false)
		task.wait()
		for _, result in pending do
			expect(result.done).to.equal(true)
			expect(result.grants.Connected).to.equal(false)
		end
		controller:setEnabled(true)
		local doomed = caller()
		local result = request(controller, doomed, { "Connected" })
		doomed:Destroy()
		task.wait()
		expect(result.done).to.equal(true)
		expect(result.grants.Connected).to.equal(false)
	end)

	it("rejects a Plugin that was destroyed before its first permission request", function()
		local controller, caller = fixture()
		local destroyed = caller()
		destroyed:Destroy()
		local result = request(controller, destroyed, { "Connected" })
		expect(result.done and result.ok == false).to.equal(true)
		expect(controller.active).to.equal(nil)
	end)

	it("cancels a caller's pending connection on revoke without affecting another granted caller", function()
		local controller, caller, bridge = fixture()
		local cancelled = 0
		function bridge:connect()
			return Promise.new(function(_, _, onCancel)
				onCancel(function()
					cancelled += 1
				end)
			end)
		end
		local first = caller()
		local client = grant(controller, first, { "ConnectAsync" })
		local other = grant(controller, caller(), { "Connected" })
		local done
		task.spawn(function()
			done = pcall(client.ConnectAsync, client)
		end)
		controller:revoke(controller.records[first])
		task.wait()
		expect(done).to.equal(false)
		expect(cancelled).to.equal(1)
		expect(other.Connected).to.equal(false)
	end)

	it("filters Changed events by readable fields and revokes old handles without firing authority", function()
		local controller, caller, bridge = fixture()
		local pluginObject = caller()
		local client = grant(controller, pluginObject, { "Connected", "Changed" })
		local events = {}
		local connection = client.Changed:Connect(function(field, value, old)
			table.insert(events, { field, value, old })
		end)
		bridge:emit({ Connected = true, Address = "secret:9000", ProjectName = "Hidden" })
		task.wait()
		expect(#events).to.equal(1)
		expect(events[1][1]).to.equal("Connected")
		expect(events[1][2]).to.equal(true)
		expect(events[1][3]).to.equal(false)
		expect(client.Changed.Fire).to.equal(nil)
		connection:Disconnect()
		bridge:emit({ Connected = false })
		task.wait()
		expect(#events).to.equal(1)
		controller:revoke(controller.records[pluginObject])
		expect(pcall(function()
			return client.Connected
		end)).to.equal(false)
		local replacement = grant(controller, pluginObject, { "Connected" })
		expect(replacement.Connected).to.equal(false)
		expect(pcall(function()
			return client.Connected
		end)).to.equal(false)
	end)

	it("keeps duplicate event subscriptions independent and cancels Changed Wait on revocation", function()
		local controller, caller, bridge = fixture()
		local pluginObject = caller()
		local client = grant(controller, pluginObject, { "Changed", "Connected" })
		local count = 0
		local function callback()
			count += 1
		end
		local first = client.Changed:Connect(callback)
		client.Changed:Connect(callback)
		bridge:emit({ Connected = true })
		task.wait()
		expect(count).to.equal(2)
		first:Disconnect()
		bridge:emit({ Connected = false })
		task.wait()
		expect(count).to.equal(3)
		local waited
		task.spawn(function()
			waited = pcall(client.Changed.Wait, client.Changed)
		end)
		controller:revoke(controller.records[pluginObject])
		task.wait()
		expect(waited).to.equal(false)
	end)

	it("denies actual permission timeout and preference disable revokes discovery and old clients", function()
		local controller, caller, _, root = fixture()
		controller.permissionTimeout = 0.01
		local result = request(controller, caller(), { "Connected" })
		task.wait(0.04)
		expect(result.done and result.grants.Connected == false).to.equal(true)
		local client = grant(controller, caller(), { "Connected" })
		Settings:set("enableHeadlessAPI", false)
		task.wait()
		expect(root:FindFirstChild("Rojo")).to.equal(nil)
		expect(pcall(function()
			return client.Connected
		end)).to.equal(false)
		Settings:set("enableHeadlessAPI", true)
		task.wait()
		expect(root:FindFirstChild("Rojo") ~= nil).to.equal(true)
	end)

	it("validates registered preferences and connection restrictions and labels bounded notifications", function()
		local controller, caller, bridge, _, notifications = fixture()
		local client = grant(controller, caller("Companion"), { "GetSetting", "SetSetting", "Notify" })
		expect(pcall(client.GetSetting, client, "priorEndpoints")).to.equal(false)
		expect(pcall(client.SetSetting, client, "autoReconnect", "true")).to.equal(false)
		expect(pcall(client.SetSetting, client, "logLevel", "Unknown")).to.equal(false)
		expect(pcall(client.SetSetting, client, "largeChangesConfirmationThreshold", 0)).to.equal(false)
		expect(pcall(client.SetSetting, client, "confirmationBehavior", "Unknown")).to.equal(false)
		expect(pcall(client.SetSetting, client, "syncReminderMode", "Unknown")).to.equal(false)
		bridge.app.serveSession = {}
		expect(pcall(client.SetSetting, client, "twoWaySync", true)).to.equal(false)
		bridge.app.serveSession = nil
		local old = Settings:get("autoReconnect")
		client:SetSetting("autoReconnect", not old)
		expect(client:GetSetting("autoReconnect")).to.equal(not old)
		Settings:set("autoReconnect", old)
		local dismiss = client:Notify({ text = "Message", timeout = 3 })
		expect(notifications[1].text).to.equal("[Plugin Companion] Message")
		dismiss()
		dismiss()
		expect(notifications[1].dismissed).to.equal(true)
		expect(pcall(client.Notify, client, { text = "Message", timeout = math.huge })).to.equal(false)
		expect(pcall(client.Notify, client, { text = string.rep("x", 2049) })).to.equal(false)
	end)

	it("closes independent context requests on revocation without exposing private state", function()
		local ApiContext = require(script.Parent.ApiContext)
		local oldNew = ApiContext.new
		local closed, cancelled, packets = 0, 0, 0
		local handlers
		ApiContext.new = function(url)
			expect(url).to.equal("http://localhost:34872")
			return {
				disconnect = function()
					closed += 1
				end,
				connect = function()
					return Promise.new(function(_, _, onCancel)
						onCancel(function()
							cancelled += 1
						end)
					end)
				end,
				connectWebSocket = function(_, callbacks)
					handlers = callbacks
					return Promise.new(function(_, _, onCancel)
						onCancel(function()
							cancelled += 1
						end)
					end)
				end,
			}
		end
		local ok, err = pcall(function()
			local controller, caller = fixture()
			local pluginObject = caller()
			local client = grant(controller, pluginObject, { "CreateApiContext" })
			for _, url in
				{
					"file:///tmp",
					"http://user:pass@localhost",
					"http://localhost/#fragment",
					"http://localhost:99999",
					"http://localhost?query",
				}
			do
				expect(pcall(client.CreateApiContext, client, url)).to.equal(false)
			end
			local context = client:CreateApiContext("http://localhost:34872")
			expect(context.__sessionId).to.equal(nil)
			expect(table.isfrozen(context)).to.equal(true)
			context:connect()
			context:connectWebSocket({
				messages = function()
					packets += 1
				end,
			})
			handlers.messages({})
			controller:revoke(controller.records[pluginObject])
			task.wait()
			expect(closed).to.equal(1)
			expect(cancelled).to.equal(2)
			handlers.messages({})
			expect(packets).to.equal(1)
			expect(pcall(context.connect, context)).to.equal(false)
		end)
		ApiContext.new = oldNew
		assert(ok, tostring(err))
	end)
	it("provides the approved caller-scoped discovery controller", function()
		local definition = script.Parent:FindFirstChild("Headless")
		expect(definition ~= nil).to.equal(true)
		expect(type(Headless.new)).to.equal("function")
	end)
	it("observes readiness independently of the UI status callback", function()
		local ServeSession = require(script.Parent.ServeSession)
		local session = ServeSession.new({
			apiContext = {
				connect = function()
					return Promise.new(function() end)
				end,
				disconnect = function() end,
			},
			twoWaySync = false,
		})
		expect(type(session.observeReady)).to.equal("function")
		session:stop()
	end)
end
