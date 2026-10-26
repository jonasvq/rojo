-- Edit-only data/module smoke test; no Run or Play session is started.
local greetingModule = game:GetService("ReplicatedStorage").Shared.Greeting
local Greeting = require(greetingModule)
assert(Greeting.message("agent") == "Hello, agent!", "shared module output differs")
assert(not pcall(Greeting.message, 42), "shared module accepted an invalid argument")
assert(greetingModule:GetAttribute("Purpose") == "AgentWorkflow", "metadata was not imported")
local server = game:GetService("ServerScriptService").Game.Boot
local player = game:GetService("StarterPlayer")
local client = player.StarterPlayerScripts.Client.Boot
assert(server:IsA("Script") and server.RunContext == Enum.RunContext.Server, "server mapping differs")
assert(client:IsA("Script") and client.RunContext == Enum.RunContext.Client, "client mapping differs")
assert(player:FindFirstChildOfClass("StarterCharacterScripts"), "character container missing")
local model = workspace.Demo
assert(model.PrimaryPart == model.Pad, "model reference did not resolve")
assert(model.Pad.Anchored and model.Pad.Size == Vector3.new(12, 1, 12), "model geometry differs")
print("AGENT_WORKFLOW_VERIFIED modules/metadata/contexts/containers/model/reference; Edit only")
