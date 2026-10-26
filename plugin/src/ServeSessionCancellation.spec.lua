return function()
	local Promise = require(script.Parent.Parent.Packages.Promise)
	local ServeSession = require(script.Parent.ServeSession)
	local PatchSet = require(script.Parent.PatchSet)

	local function fixture(delayConnect, delayRead, output)
		local info = { sessionId = "cancel-test", rootInstanceId = "ROOT", projectName = "Cancel" }
		if output then
			info.capabilities = { "studioOutputV1" }
			info.outputLogs = { "Error" }
		end
		local finishConnect, finishRead
		local calls = { connect = 0, read = 0, hydrate = 0, apply = 0, socket = 0, disconnect = 0, cancelled = 0 }
		local api = {}
		function api:connect()
			calls.connect += 1
			if not delayConnect then
				return Promise.resolve(info)
			end
			return Promise.new(function(resolve, _, onCancel)
				onCancel(function()
					calls.cancelled += 1
				end)
				finishConnect = function()
					resolve(info)
				end
			end)
		end
		function api:read()
			calls.read += 1
			local body = { instances = {}, messageCursor = 0 }
			if not delayRead then
				return Promise.resolve(body)
			end
			return Promise.new(function(resolve, _, onCancel)
				onCancel(function()
					calls.cancelled += 1
				end)
				finishRead = function()
					resolve(body)
				end
			end)
		end
		function api:setMessageCursor() end
		function api:connectWebSocket()
			calls.socket += 1
			return Promise.new(function() end)
		end
		function api:disconnect()
			calls.disconnect += 1
		end
		local session = ServeSession.new({ apiContext = api, twoWaySync = false })
		session.__reconciler.hydrate = function()
			calls.hydrate += 1
		end
		session.__reconciler.diff = function()
			return true, PatchSet.newEmpty()
		end
		session.__applyPatch = function()
			calls.apply += 1
		end
		return session,
			calls,
			function()
				if finishConnect then
					finishConnect()
				end
				if finishRead then
					finishRead()
				end
			end
	end

	it("starts negotiated output only after acceptance and stops it with the session", function()
		local session, _, finish = fixture(true, false, true)
		session:start()
		expect(session.__studioOutput).to.equal(nil)
		finish()
		task.wait()
		local collector = session.__studioOutput
		expect(collector ~= nil and collector.__started).to.equal(true)
		local connection = collector.__connection
		session:stop()
		expect(session.__studioOutput).to.equal(nil)
		expect(collector.__started).to.equal(false)
		expect(connection.Connected).to.equal(false)
	end)

	it("removes the output collector when the connected place is rejected", function()
		local session = fixture(false, false, true)
		session:start()
		local collector = session.__studioOutput
		expect(collector ~= nil and collector.__started).to.equal(true)
		session.__serverInfo.unexpectedPlaceIds = { game.PlaceId }
		expect(session:__ensurePlaceAllowed()).to.equal(false)
		expect(session.__studioOutput).to.equal(nil)
		expect(collector.__started).to.equal(false)
	end)

	it("does not create output listeners after a cancelled handshake or rejected connection", function()
		for _, delayConnect in { true, false } do
			local session, _, finish = fixture(delayConnect, false, true)
			if not delayConnect then
				session:onStatusChanged(function(status)
					if status == ServeSession.Status.Connected then
						session:stop()
					end
				end)
			end
			session:start()
			session:stop()
			finish()
			task.wait()
			expect(session.__studioOutput).to.equal(nil)
		end
	end)

	it("does not open a socket when stopped by the connected callback", function()
		local session, calls = fixture(false, false)
		session:onStatusChanged(function(status)
			if status == ServeSession.Status.Connected then
				session:stop()
			end
		end)
		session:start()
		task.wait()
		expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
		expect(calls.socket).to.equal(0)
	end)

	it("cancels pending startup work and disconnects only once", function()
		for _, delayConnect in { true, false } do
			local session, calls = fixture(delayConnect, not delayConnect)
			session:start()
			session:stop()
			session:stop()
			task.wait()
			expect(calls.cancelled).to.equal(1)
			expect(calls.disconnect).to.equal(1)
			expect(calls.apply).to.equal(0)
			expect(calls.socket).to.equal(0)
		end
	end)

	it("cancels an initial confirmation waiter without a late commit", function()
		local decision = Instance.new("BindableEvent")
		local session, calls = fixture(false, false)
		local waiting = false
		session:setConfirmCallback(function()
			waiting = true
			return decision.Event:Wait()
		end)
		session:start()
		expect(waiting).to.equal(true)
		session:stop()
		task.wait()
		decision:Fire("Accept")
		task.wait()
		expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
		expect(calls.apply).to.equal(0)
		expect(calls.socket).to.equal(0)
		decision:Destroy()
	end)

	it("does not resume or connect after cancellation before the handshake completes", function()
		local session, calls, finish = fixture(true, false)
		session:start()
		session:stop()
		finish()
		task.wait()
		expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
		expect(calls.read).to.equal(0)
		expect(calls.hydrate).to.equal(0)
		expect(calls.apply).to.equal(0)
		expect(calls.socket).to.equal(0)
		session:stop()
	end)

	it("does not hydrate or apply a read that completes after cancellation", function()
		local session, calls, finish = fixture(false, true)
		session:start()
		expect(calls.read).to.equal(1)
		session:stop()
		finish()
		task.wait()
		expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
		expect(calls.hydrate).to.equal(0)
		expect(calls.apply).to.equal(0)
		expect(calls.socket).to.equal(0)
		session:stop()
	end)

	it("does not apply or reconnect when stopped inside initial confirmation", function()
		local session, calls = fixture(false, false)
		session:setConfirmCallback(function()
			session:stop()
			return "Accept"
		end)
		session:start()
		task.wait()
		expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
		expect(calls.apply).to.equal(0)
		expect(calls.socket).to.equal(0)
		session:stop()
	end)
end
