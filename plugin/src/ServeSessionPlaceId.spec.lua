return function()
	local Promise = require(script.Parent.Parent.Packages.Promise)
	local ServeSession = require(script.Parent.ServeSession)
	local PatchSet = require(script.Parent.PatchSet)

	local function fixture(info)
		local callbacks, finishSocket, detail
		local applied = 0
		local api = {}
		function api:connect()
			return Promise.resolve(info)
		end
		function api:read()
			return Promise.resolve({ instances = {}, messageCursor = 0 })
		end
		function api:setMessageCursor() end
		function api:connectWebSocket(handlers)
			callbacks = handlers
			return Promise.new(function(resolve)
				finishSocket = resolve
			end)
		end
		function api:disconnect()
			if finishSocket then
				finishSocket()
			end
		end
		local session = ServeSession.new({ apiContext = api, twoWaySync = false })
		session.__reconciler.hydrate = function() end
		session.__reconciler.diff = function()
			return true, PatchSet.newEmpty()
		end
		session.__applyPatch = function()
			applied += 1
		end
		session:onStatusChanged(function(_, reason)
			detail = reason
		end)
		session:start()
		expect(session:getStatus()).to.equal(ServeSession.Status.Connected)
		return session,
			function()
				callbacks.messages({ messages = { PatchSet.newEmpty() }, messageCursor = 1 })
			end,
			function()
				return applied, detail
			end
	end

	it("disconnects and rejects pending incoming patches when Studio changes to a disallowed place", function()
		local previous = game.PlaceId
		local nextId = previous + 1
		local session, incoming, result = fixture({
			sessionId = "place-change",
			rootInstanceId = "ROOT",
			projectName = "Allowed",
			expectedPlaceIds = {
				previous,
			},
		})
		local before = result()
		local ok, err = pcall(function()
			game:SetPlaceId(nextId)
			incoming()
			task.wait()
			expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
			local after, detail = result()
			expect(after).to.equal(before)
			expect(string.find(tostring(detail), "place", 1, true) ~= nil).to.equal(true)
		end)
		session:stop()
		game:SetPlaceId(previous)
		assert(ok, tostring(err))
	end)

	it("disconnects on a newly blocked ID even without an incoming patch", function()
		local previous = game.PlaceId
		local nextId = previous + 1
		local session, _, result = fixture({
			sessionId = "blocked-place",
			rootInstanceId = "ROOT",
			projectName = "Blocked",
			unexpectedPlaceIds = { nextId },
		})
		local ok, err = pcall(function()
			game:SetPlaceId(nextId)
			task.wait()
			expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
			local _, detail = result()
			expect(string.find(tostring(detail), "blockedPlaceIds", 1, true) ~= nil).to.equal(true)
		end)
		session:stop()
		game:SetPlaceId(previous)
		assert(ok, tostring(err))
	end)

	it("keeps allowed and unrestricted place changes connected", function()
		local previous = game.PlaceId
		local nextId = previous + 1
		for _, restricted in { true, false } do
			local info = { sessionId = "allowed-place", rootInstanceId = "ROOT", projectName = "Allowed" }
			if restricted then
				info.expectedPlaceIds = { previous, nextId }
			end
			local session, incoming, result = fixture(info)
			local before = result()
			local ok, err = pcall(function()
				game:SetPlaceId(nextId)
				task.wait()
				expect(session:getStatus()).to.equal(ServeSession.Status.Connected)
				incoming()
				expect(result()).to.equal(before + 1)
			end)
			session:stop()
			game:SetPlaceId(previous)
			assert(ok, tostring(err))
		end
	end)

	it("checks again after a precommit hook changes place identity", function()
		local previous = game.PlaceId
		local session = fixture({
			sessionId = "hook-place",
			rootInstanceId = "ROOT",
			projectName = "Hook",
			expectedPlaceIds = {
				previous,
			},
		})
		local committed = 0
		session.__applyPatch = nil
		session.__reconciler.applyPatch = function()
			committed += 1
			return PatchSet.newEmpty()
		end
		session:hookPrecommit(function()
			game:SetPlaceId(previous + 1)
		end)
		local ok, err = pcall(function()
			session:__applyPatch(PatchSet.newEmpty())
			expect(committed).to.equal(0)
			expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
		end)
		session:stop()
		game:SetPlaceId(previous)
		assert(ok, tostring(err))
	end)
end
