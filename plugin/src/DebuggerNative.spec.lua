return function()
	local Native = require(script.Parent.DebuggerNative)
	local function desired(instance)
		return {
			generation = 1,
			source = {
				instanceId = "OWNED",
				className = instance.ClassName,
				source = instance.Source,
				sourceHash = "HASH",
			},
			plan = {
				points = { 4 },
				actions = { "Into", "Over", "Out", "Resume" },
				maxHits = 4,
				tableDepth = 2,
				evaluations = { { identifier = "localCounter", arithmetic = { operator = "Add", operand = 1 } } },
			},
		}
	end
	local function fixture(callback)
		local instance = Instance.new("ModuleScript")
		instance.Name = "RojoOwnedMockDebugger"
		instance.Source = "local localCounter = 7\nreturn localCounter\n"
		instance.Parent = game:GetService("ServerStorage")
		local rows, removed = {}, {}
		local service = {
			variables = {
				{ Name = "localCounter", Type = "double", Value = "7", Scope = Enum.ScriptVariableScope.Local },
				{
					Name = "nested",
					Type = "LazyTable",
					Value = "table summary",
					VariablesReference = 55,
					Scope = Enum.ScriptVariableScope.Local,
				},
				{ Name = "_G", Type = "table", Value = "hidden global", Scope = Enum.ScriptVariableScope.Global },
			},
		}
		function service:AddBreakpoint(target, point)
			assert(
				target == instance and point.Condition == nil and point.LogMessage == nil,
				"Only typed owned breakpoints"
			)
			return { Verified = true, Line = point.Line + 1 }
		end
		function service:RemoveBreakpoint(target, line)
			assert(target == instance, "Remove only owned script")
			table.insert(removed, line)
		end
		function service:GetStackTrace(thread, startFrame)
			assert(thread == 91 and startFrame == nil, "Use native thread and omitted stack index")
			return { Frames = { { Id = 5, ScriptPath = instance:GetFullName(), Line = 5 } } }
		end
		function service:GetRootVariables(frame)
			assert(frame == 5, "Owned frame")
			return self.variables
		end
		function service:GetVariables(reference)
			assert(reference == 55, "Use variables reference")
			return { { Name = "value", Type = "double", Value = "3" } }
		end
		function service:Evaluate(expression, frame)
			assert(frame == 5 and expression == "localCounter + (1)", "Only generated numeric expression")
			return { Type = "double", Result = "8" }
		end
		local native = Native.new(service, function(plan, row)
			table.insert(rows, { desired = plan, report = row })
		end)
		local ok, err = pcall(callback, native, service, instance, rows, removed)
		native:stop()
		instance:Destroy()
		assert(ok, tostring(err))
	end

	it("never installs before consent and returns native step tuples with bounded immutable values", function()
		fixture(function(native, service, instance, rows, removed)
			expect(service.OnStopped).to.equal(nil)
			expect(pcall(native.arm, native, desired(instance), instance)).to.equal(false)
			native:install()
			native:arm(desired(instance), instance)
			for _, expected in
				{
					Enum.DebuggerResumeType.StepInto,
					Enum.DebuggerResumeType.StepOver,
					Enum.DebuggerResumeType.StepOut,
					Enum.DebuggerResumeType.Resume,
				}
			do
				local result =
					table.pack(service.OnStopped({ Reason = Enum.ScriptStoppedReason.Breakpoint, ThreadIds = { 91 } }))
				expect(result[1]).to.equal(expected)
				expect(result.n).to.equal(if expected == Enum.DebuggerResumeType.Resume then 1 else 2)
				if result.n == 2 then
					expect(result[2]).to.equal(91)
				end
			end
			expect(#rows).to.equal(4)
			expect(rows[1].report.requestedLine).to.equal(4)
			expect(rows[1].report.verifiedLine).to.equal(5)
			expect(rows[1].report.evaluations[1].value).to.equal("8")
			expect(#rows[1].report.frames[1].values).to.equal(2)
			expect(rows[1].report.frames[1].values[2].children[1].value).to.equal("3")
			service.variables[1].Value = "99"
			expect(rows[1].report.frames[1].values[1].value).to.equal("7")
			expect(#removed).to.equal(1)
			expect(removed[1]).to.equal(5)
			expect(next(native.entries)).to.equal(nil)
			expect(service.OnStopped({ Reason = Enum.ScriptStoppedReason.Breakpoint, ThreadIds = { 91 } })).to.equal(
				Enum.DebuggerResumeType.Resume
			)
			expect(#rows).to.equal(4)
			native:stop()
			expect(service.OnStopped).to.equal(nil)
		end)
	end)

	it("resumes foreign/manual stops and guards native failures without global clearing", function()
		fixture(function(native, service, instance, rows, removed)
			native:install()
			native:arm(desired(instance), instance)
			service.GetStackTrace = function()
				return { Frames = { { Id = 5, ScriptPath = "Foreign.Script", Line = 5 } } }
			end
			expect(service.OnStopped({ Reason = Enum.ScriptStoppedReason.Breakpoint, ThreadIds = { 91 } })).to.equal(
				Enum.DebuggerResumeType.Resume
			)
			expect(#rows).to.equal(0)
			expect(#removed).to.equal(0)
			expect(native.diagnostics.foreign).to.equal(1)
			native.onStopped = function()
				error("Native inspection failed")
			end
			expect(service.OnStopped({ Reason = Enum.ScriptStoppedReason.Breakpoint, ThreadIds = { 91 } })).to.equal(
				Enum.DebuggerResumeType.Resume
			)
			expect(#removed).to.equal(1)
			expect(native.diagnostics.error:find("Native inspection failed", 1, true) ~= nil).to.equal(true)
		end)
	end)

	it("attributes the measured next-instruction stack line to the installed breakpoint", function()
		fixture(function(native, service, instance, rows)
			service.AddBreakpoint = function(_, _, point)
				return { Verified = true, Line = point.Line }
			end
			native:install()
			native:arm(desired(instance), instance)
			local action = service.OnStopped({ Reason = Enum.ScriptStoppedReason.Breakpoint, ThreadIds = { 91 } })
			expect(action).to.equal(Enum.DebuggerResumeType.StepInto)
			expect(rows[1].report.verifiedLine).to.equal(4)
			expect(rows[1].report.frames[1].line).to.equal(5)
		end)
	end)

	it("resumes exceptions at an installed line without treating them as breakpoint hits", function()
		fixture(function(native, service, instance, rows)
			native:install()
			native:arm(desired(instance), instance)
			expect(service.OnStopped({ Reason = Enum.ScriptStoppedReason.Exception, ThreadIds = { 91 } })).to.equal(
				Enum.DebuggerResumeType.Resume
			)
			expect(#rows).to.equal(0)
		end)
	end)

	it("consumes filtered hits and removes points at the stop budget", function()
		fixture(function(native, service, instance, rows, removed)
			local plan = desired(instance)
			plan.plan.maxHits = 2
			plan.plan.filter = { identifier = "localCounter", operator = "Gt", value = 100 }
			native:install()
			native:arm(plan, instance)
			service.OnStopped({ Reason = Enum.ScriptStoppedReason.Breakpoint, ThreadIds = { 91 } })
			service.OnStopped({ Reason = Enum.ScriptStoppedReason.Breakpoint, ThreadIds = { 91 } })
			expect(#rows).to.equal(0)
			expect(#removed).to.equal(1)
		end)
	end)

	it("reports ambiguous, optimized, nonnumeric and native evaluation errors honestly", function()
		for _, variable in
			{
				false,
				{ Name = "localCounter", Type = "string", Value = "7" },
				{ Name = "localCounter", Type = "double", Value = "*** Value not found ***" },
				"duplicate",
				"nativeError",
			}
		do
			fixture(function(native, service, instance, rows)
				if variable == false then
					service.variables = {}
				elseif type(variable) == "table" then
					service.variables = { variable }
				elseif variable == "duplicate" then
					table.insert(service.variables, table.clone(service.variables[1]))
				else
					service.Evaluate = function()
						return { Type = "double", Result = "*** Value not found ***" }
					end
				end
				native:install()
				native:arm(desired(instance), instance)
				service.OnStopped({ Reason = Enum.ScriptStoppedReason.Breakpoint, ThreadIds = { 91 } })
				expect(rows[1].report.evaluations[1].kind).to.equal("error")
			end)
		end
	end)

	it("rejects caller code, division by zero and excessive native requests", function()
		fixture(function(_, _, instance)
			for _, name in { "end", "x.y", "x[1]", "print(1)", "a;error(1)", "", "1x" } do
				local plan = desired(instance).plan
				plan.evaluations[1].identifier = name
				expect(Native.validate(plan)).to.equal(false)
			end
			local plan = desired(instance).plan
			plan.evaluations[1].arithmetic = { operator = "Divide", operand = 0 }
			expect(Native.validate(plan)).to.equal(false)
			plan = desired(instance).plan
			plan.maxHits = 65
			expect(Native.validate(plan)).to.equal(false)
			plan = desired(instance).plan
			plan.actions = { "Pause" }
			expect(Native.validate(plan)).to.equal(false)
		end)
	end)

	it("rejects changed native source and removes only its installed lines on replacement", function()
		fixture(function(native, service, instance, rows, removed)
			local plan = desired(instance)
			plan.source.source = "different"
			native:install()
			expect(pcall(native.arm, native, plan, instance)).to.equal(false)
			native:arm(desired(instance), instance)
			instance.Source = "return 'changed'"
			task.wait()
			expect(#removed).to.equal(1)
			expect(next(native.entries)).to.equal(nil)
			expect(service.OnStopped({ Reason = Enum.ScriptStoppedReason.Breakpoint, ThreadIds = { 91 } })).to.equal(
				Enum.DebuggerResumeType.Resume
			)
			expect(#rows).to.equal(0)
		end)
	end)
end
