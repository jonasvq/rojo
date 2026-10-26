return function()
	local Promise = require(script.Parent.Parent.Packages.Promise)
	local Plans = require(script.Parent.DebuggerPlans)
	local function session()
		return {
			__serverInfo = { rootInstanceId = "ROOT", capabilities = { "debuggerPlansV1" } },
			__instanceMap = { fromIds = {} },
			__apiContext = {
				__connected = true,
				__socketOpen = true,
				__debuggerRelay = function()
					return Promise.resolve({ rootId = "ROOT", plans = {} })
				end,
			},
		}
	end
	it("requires capability and never obtains the native service in Edit or unsupported contexts", function()
		expect(Plans.supported({ __serverInfo = { capabilities = {} } })).to.equal(false)
		local calls = 0
		local controller = Plans.new(session(), function() end, function()
			calls += 1
			error("Must not acquire")
		end)
		for _, mode in { "Edit", "Unsupported" } do
			controller.__mode = function()
				return mode
			end
			controller:__reconcile({ rootId = "ROOT", plans = {} })
		end
		expect(calls).to.equal(0)
		expect(controller.enabled).to.equal(false)
	end)
	it("requires readiness and cancels late permission replies before any native control", function()
		local current = session()
		local resolve, cancelled
		current.__apiContext.__debuggerRelay = function(_, body)
			if body and body.operation == "disable" then
				return Promise.resolve({})
			end
			return Promise.new(function(done, _, onCancel)
				resolve = done
				onCancel(function()
					cancelled = true
				end)
			end)
		end
		local calls = 0
		local controller = Plans.new(current, function() end, function()
			calls += 1
			error("Must not acquire")
		end)
		current.__apiContext.__socketOpen = false
		expect(pcall(controller.enable, controller)).to.equal(false)
		current.__apiContext.__socketOpen = true
		controller:enable()
		expect(controller.enabled).to.equal(true)
		controller:stop()
		resolve({ rootId = "ROOT", plans = {} })
		task.wait()
		expect(controller.enabled).to.equal(false)
		expect(controller.timer).to.equal(nil)
		expect(controller.modeConnection).to.equal(nil)
		expect(calls).to.equal(0)
		-- Resolution and cancellation may race; neither may arm the adapter.
		expect(cancelled == true or controller.request == nil).to.equal(true)
	end)
	it("bounds the immutable stop queue and clears it on revocation", function()
		local controller = Plans.new(session(), function() end)
		controller.enabled = true
		controller.relayId = "RELAY"
		local desired =
			{ source = { instanceId = "SOURCE", sourceHash = "HASH" }, generation = 1, plan = { maxHits = 64 } }
		for i = 1, 70 do
			controller:__enqueue(desired, { hit = i })
		end
		expect(#controller.queue).to.equal(64)
		expect(controller.queue[1].report.hit).to.equal(7)
		expect(controller.queue[#controller.queue].report.overflow).to.equal(6)
		controller:stop()
		expect(#controller.queue).to.equal(0)
	end)
	it("checks private relay session acknowledgements and cancels outstanding requests", function()
		local Http = require(script.Parent.Parent.Packages.Http)
		local ApiContext = require(script.Parent.ApiContext)
		local oldGet, oldPost, oldMsgpack = Http.get, Http.post, Http.Response.msgpack
		local ok, err = pcall(function()
			local api = ApiContext.new("http://test.invalid")
			api.__sessionId = "SESSION"
			local response = { sessionId = "SESSION", plans = {} }
			Http.Response.msgpack = function(value)
				return value.payload
			end
			Http.get = function(url)
				expect(url).to.equal("http://test.invalid/api/debugger/relay?sessionId=SESSION&relayId=RELAY")
				return Promise.resolve({ code = 200, payload = response })
			end
			Http.post = function(url, body)
				expect(url).to.equal("http://test.invalid/api/debugger/relay")
				expect(body).to.equal(
					Http.msgpackEncode({ operation = "disable", relayId = "RELAY", sessionId = "SESSION" })
				)
				return Promise.resolve({ code = 200, payload = response })
			end
			expect(api:__debuggerRelay(nil, "RELAY"):await()).to.equal(true)
			expect(api:__debuggerRelay({ operation = "disable", relayId = "RELAY" }):await()).to.equal(true)
			response = { sessionId = "WRONG" }
			expect(api:__debuggerRelay(nil, "RELAY"):await()).to.equal(false)
			api:disconnect()
			expect(api:__debuggerRelay(nil, "RELAY"):await()).to.equal(false)
		end)
		Http.get, Http.post, Http.Response.msgpack = oldGet, oldPost, oldMsgpack
		assert(ok, tostring(err))
	end)
end
