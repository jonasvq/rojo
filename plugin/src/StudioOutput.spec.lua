return function()
	local Promise = require(script.Parent.Parent.Packages.Promise)
	local StudioOutput = require(script.Parent.StudioOutput)
	it("posts MessagePack to the current session and rejects invalid acknowledgements", function()
		local Http = require(script.Parent.Parent.Packages.Http)
		local ApiContext = require(script.Parent.ApiContext)
		local oldPost, oldMsgpack = Http.post, Http.Response.msgpack
		local ok, err = pcall(function()
			local api = ApiContext.new("http://test.invalid")
			api.__sessionId = "SESSION"
			local ack = { sessionId = "SESSION" }
			Http.Response.msgpack = function(response)
				return response.payload
			end
			Http.post = function(url, body)
				expect(url).to.equal("http://test.invalid/api/output")
				expect(body).to.equal(
					Http.msgpackEncode({ sessionId = "SESSION", entries = { { level = "Error", message = "failure" } } })
				)
				return Promise.resolve({ code = 200, payload = ack })
			end
			expect(api:output({ { level = "Error", message = "failure" } }):await()).to.equal(true)
			ack = { sessionId = "OLD" }
			expect(api:output({ { level = "Error", message = "failure" } }):await()).to.equal(false)
			ack = false
			expect(api:output({ { level = "Error", message = "failure" } }):await()).to.equal(false)
			api:disconnect()
		end)
		Http.post, Http.Response.msgpack = oldPost, oldMsgpack
		assert(ok, tostring(err))
	end)
	local function fixture(levels, request)
		local batches = {}
		local collector = StudioOutput.new({
			output = function(_, entries)
				table.insert(batches, entries)
				return if request then request() else Promise.resolve({})
			end,
		}, levels)
		-- Most assertions drive the queue synchronously, without a global listener
		-- that could accidentally consume the test runner's diagnostics.
		collector.__started = true
		collector.__schedule = function() end
		return collector, batches
	end

	it("requires explicit valid capability and level negotiation", function()
		for _, info in
			{
				{},
				{ outputLogs = { "Error" } },
				{ capabilities = { "studioOutputV1" } },
				{ capabilities = { "studioOutputV1" }, outputLogs = {} },
				{ capabilities = { "studioOutputV1" }, outputLogs = { "Unknown" } },
				{ capabilities = { "studioOutputV1" }, outputLogs = { [1] = "Error", [3] = "Info" } },
				{ capabilities = { "studioOutputV1" }, outputLogs = { Error = true } },
			}
		do
			expect(StudioOutput.fromServerInfo({}, info)).to.equal(nil)
		end
		expect(
			StudioOutput.fromServerInfo({}, { capabilities = { "studioOutputV1" }, outputLogs = { "Error" } }) ~= nil
		).to.equal(true)
	end)

	it("selects severity with bounded separate stack context and excludes Rojo diagnostics", function()
		local collector = fixture({ "Error" })
		local function capture(message, kind)
			collector:__capture(message, kind or Enum.MessageType.MessageInfo)
		end
		capture("error", Enum.MessageType.MessageError)
		capture("Stack Begin")
		capture("Script 'ReplicatedFirst.runtime', Line 27")
		capture("Stack End")
		capture("unrelated info")
		capture("warning", Enum.MessageType.MessageWarning)
		capture("   [Rojo-Warn] transport failed", Enum.MessageType.MessageError)
		expect(#collector.__queue).to.equal(4)
		for _, entry in collector.__queue do
			expect(entry.level).to.equal("Error")
		end
		capture("expiry", Enum.MessageType.MessageError)
		collector.__pendingTime = os.clock() - 2
		capture("Stack Begin")
		capture("Script 'Expired', Line 1")
		expect(#collector.__queue).to.equal(5)
		capture("bounded", Enum.MessageType.MessageError)
		capture("Stack Begin")
		for _ = 1, 80 do
			capture("Script 'Frame', Line 1")
		end
		expect(#collector.__queue).to.equal(70)
		collector:stop()
	end)

	it("bounds UTF-8 messages and drops oldest entries before single-flight batches", function()
		local finish
		local collector, batches = fixture({ "Output" }, function()
			return Promise.new(function(resolve)
				finish = resolve
			end)
		end)
		collector:__capture(string.rep(utf8.char(0x1f600), 3000), Enum.MessageType.MessageOutput)
		local message = collector.__queue[1].message
		expect(#message <= 8192 and utf8.len(message) ~= nil).to.equal(true)
		expect(message:sub(-12)).to.equal(" [truncated]")
		collector:__capture(string.char(255), Enum.MessageType.MessageOutput)
		expect(#collector.__queue).to.equal(1)
		table.clear(collector.__queue)
		for i = 1, 300 do
			collector:__capture(tostring(i), Enum.MessageType.MessageOutput)
		end
		expect(#collector.__queue).to.equal(256)
		expect(collector.__dropped).to.equal(44)
		collector:__flush(0)
		expect(#batches).to.equal(1)
		expect(#batches[1]).to.equal(64)
		expect(batches[1][1].message).to.equal("45")
		collector:__flush(0)
		expect(#batches).to.equal(1)
		finish({})
		task.wait()
		collector:__flush(0)
		expect(#batches).to.equal(2)
		collector:stop()
	end)

	it("drops failed batches and cancels in-flight work without late-session effects", function()
		local cancelled = 0
		local finish, reject
		local collector, batches = fixture({ "Error" }, function()
			return Promise.new(function(resolve, fail, onCancel)
				finish, reject = resolve, fail
				onCancel(function()
					cancelled += 1
				end)
			end)
		end)
		collector:__capture("first", Enum.MessageType.MessageError)
		collector:__flush(0)
		reject("failure")
		task.wait()
		expect(collector.__inflight).to.equal(false)
		expect(#collector.__queue).to.equal(0)
		collector:__capture("second", Enum.MessageType.MessageError)
		collector:__flush(0)
		collector:stop()
		finish({})
		task.wait()
		expect(cancelled).to.equal(1)
		expect(#batches).to.equal(2)
		expect(collector.__inflight).to.equal(false)
		expect(#collector.__queue).to.equal(0)
	end)

	it("stops before delayed flush and starts only one native listener", function()
		local count = 0
		local collector = StudioOutput.new({
			output = function()
				count += 1
				return Promise.resolve({})
			end,
		}, { "Output" })
		collector:start()
		local connection = collector.__connection
		collector:start()
		expect(collector.__connection).to.equal(connection)
		collector:__capture("queued", Enum.MessageType.MessageOutput)
		expect(collector.__timer ~= nil).to.equal(true)
		collector:stop()
		task.wait(0.15)
		expect(count).to.equal(0)
		expect(connection.Connected).to.equal(false)
		collector:__capture("after stop", Enum.MessageType.MessageOutput)
		expect(#collector.__queue).to.equal(0)
	end)
	it("forwards native selected MessageOut output in a bounded batch", function()
		local definition = script.Parent:FindFirstChild("StudioOutput")
		expect(definition ~= nil).to.equal(true)
		local batches = {}
		local api = {}
		function api:output(entries)
			table.insert(batches, entries)
			return Promise.resolve({})
		end
		local collector = StudioOutput.new(api, { "Output" })
		collector:start()
		print("ROJO_NATIVE_SELECTED_OUTPUT")
		local deadline = os.clock() + 2
		repeat
			task.wait()
		until #batches > 0 or os.clock() > deadline
		collector:stop()
		expect(#batches).to.equal(1)
		expect(batches[1][1].level).to.equal("Output")
		expect(batches[1][1].message).to.equal("ROJO_NATIVE_SELECTED_OUTPUT")
	end)
end
