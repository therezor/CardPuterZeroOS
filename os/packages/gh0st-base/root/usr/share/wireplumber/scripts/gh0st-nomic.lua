-- Clients on the no-mic pulse socket see every object but sources, so an
-- app the user took the microphone from can still play (docs/isolation.md).
log = Log.open_topic ("s-gh0st-nomic")

local ACCESS = "gh0st-nomic"

local function restricted (props)
  return props["pipewire.access"] == ACCESS or props["pipewire.client.access"] == ACCESS
end

local function hidden (node)
  local c = node.properties["media.class"] or ""
  return c:find ("^Audio/Source") ~= nil or c:find ("^Stream/Input") ~= nil
      or c:find ("^Video/Source") ~= nil
end

clients_om = ObjectManager { Interest { type = "client" } }
nodes_om = ObjectManager { Interest { type = "node" } }

local function apply (client)
  local perms = { ["any"] = "all" }
  for node in nodes_om:iterate () do
    if hidden (node) then
      perms[node["bound-id"]] = "-"
    end
  end
  client:update_permissions (perms)
  log:info (client, "no mic for client " .. client["bound-id"])
end

clients_om:connect ("object-added", function (_, client)
  if restricted (client.properties) then
    apply (client)
  end
end)

nodes_om:connect ("object-added", function (_, node)
  if not hidden (node) then
    return
  end
  for client in clients_om:iterate () do
    if restricted (client.properties) then
      client:update_permissions { [node["bound-id"]] = "-" }
    end
  end
end)

nodes_om:activate ()
clients_om:activate ()
