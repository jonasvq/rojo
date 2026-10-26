return function()
	local HttpService = game:GetService("HttpService")
	local OpenLaunch = require(script.Parent.OpenLaunch)
	local roots = {}
	local function descriptor()
		return {
			version = 1,
			host = "127.0.0.1",
			port = 34872,
			sessionId = "12345678-1234-4321-8765-123456789abc",
			projectName = "Native open fixture",
			createdAt = 1000,
			expiresAt = 1600,
		}
	end
	local function fixture(details)
		local root = Instance.new("Folder")
		table.insert(roots, root)
		local marker = Instance.new("Folder")
		marker.Name = OpenLaunch.markerName
		marker.Parent = root
		local value = Instance.new("StringValue")
		value.Name = "Details"
		value.Value = HttpService:JSONEncode(details)
		value.Parent = marker
		return root, marker
	end
	afterEach(function()
		for _, root in roots do
			root:Destroy()
		end
		table.clear(roots)
	end)
	describe("rojo open launch descriptor", function()
		it("consumes the recognized marker once and freezes valid details", function()
			local root, marker = fixture(descriptor())
			local details, err = OpenLaunch.consume(root, 1100)
			expect(err).to.equal(nil)
			expect(details.sessionId).to.equal(descriptor().sessionId)
			expect(table.isfrozen(details)).to.equal(true)
			expect(marker.Parent).to.equal(nil)
			expect(OpenLaunch.consume(root, 1100)).to.equal(nil)
		end)
		it("removes stale recognized markers and rejects their connection", function()
			local root, marker = fixture(descriptor())
			local details, err = OpenLaunch.consume(root, 1601)
			expect(details).to.equal(nil)
			expect(type(err)).to.equal("string")
			expect(marker.Parent).to.equal(nil)
		end)
		it("rejects nonlocal endpoints, invalid ports, sessions and time values", function()
			for _, change in
				{
					{ "host", "0.0.0.0" },
					{ "host", "localhost.evil.test" },
					{ "port", 0 },
					{ "port", 65536 },
					{ "port", 1.5 },
					{ "port", "34872" },
					{ "sessionId", "another server" },
					{ "projectName", string.rep("x", 257) },
					{ "projectName", "hidden\nlabel" },
					{ "createdAt", 1200 },
					{ "expiresAt", 1601 },
					{ "expiresAt", math.huge },
				}
			do
				local data = descriptor()
				data[change[1]] = change[2]
				local details, err = OpenLaunch.validate(data, 1100)
				expect(details).to.equal(nil)
				expect(type(err)).to.equal("string")
			end
		end)
		it("keeps unrecognized content and duplicate markers intact", function()
			local data = descriptor()
			data.executable = "never execute"
			local root, marker = fixture(data)
			local details, err = OpenLaunch.consume(root, 1100)
			expect(details).to.equal(nil)
			expect(type(err)).to.equal("string")
			expect(marker.Parent).to.equal(root)
			marker:Destroy()
			root, marker = fixture(descriptor())
			local duplicate = Instance.new("Folder")
			duplicate.Name = OpenLaunch.markerName
			duplicate.Parent = root
			details, err = OpenLaunch.consume(root, 1100)
			expect(details).to.equal(nil)
			expect(type(err)).to.equal("string")
			expect(marker.Parent).to.equal(root)
			expect(duplicate.Parent).to.equal(root)
		end)
		it("keeps malformed and oversized descriptor content intact", function()
			for _, text in { "{invalid", string.rep("x", 4097), '{"version":2}' } do
				local root, marker = fixture(descriptor())
				marker.Details.Value = text
				local details, err = OpenLaunch.consume(root, 1100)
				expect(details).to.equal(nil)
				expect(type(err)).to.equal("string")
				expect(marker.Parent).to.equal(root)
			end
		end)
	end)
end
