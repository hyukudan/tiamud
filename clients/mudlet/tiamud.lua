Tiamud = Tiamud or {
  rooms = {},
  scene = nil,
  assets = nil,
  media = nil,
  mediaEnabled = false,
  linkMode = "plain",
  graphicsMode = "none",
  plainMode = false,
  reducedMotion = false,
  screenReader = false,
  decorativeEnabled = true,
}

local directions = {
  north = true,
  east = true,
  south = true,
  west = true,
  up = true,
  down = true,
}

local function safe_string(value, maximum)
  if type(value) ~= "string" or #value == 0 or #value > maximum then
    return nil
  end
  if value:find("[%z\1-\31\127]") then
    return nil
  end
  return value
end

local function safe_content_addressed_path(value, media_only)
  local path = safe_string(value, 256)
  if not path or path:find("[?#\\]") then return nil end
  local digest, file = path:match("^/assets/([0-9a-f]+)/([A-Za-z0-9._-]+)$")
  if not digest or #digest ~= 64 or not file or #file > 128 then return nil end
  if media_only
      and not file:match("%.opus$")
      and not file:match("%.ogg$")
      and not file:match("%.mp3$") then
    return nil
  end
  return path
end

local function rich_output_blocked()
  return Tiamud.plainMode
      or Tiamud.reducedMotion
      or Tiamud.screenReader
      or not Tiamud.decorativeEnabled
end

local function on_room_info()
  local room = gmcp and gmcp.Room and gmcp.Room.Info
  if type(room) ~= "table" then return end
  local room_id = safe_string(room.num, 128)
  local name = safe_string(room.name, 256)
  if not room_id or not name then return end

  Tiamud.rooms[room_id] = {
    name = name,
    zone = safe_string(room.zone, 128),
    exits = type(room.exits) == "table" and room.exits or {},
  }
  raiseEvent("tiamud.room", room_id, Tiamud.rooms[room_id])

  -- Local mapper updates only: no server command is ever sent. Unknown or
  -- malformed directions are ignored rather than evaluated as Lua.
  if type(createRoomID) ~= "function" then return end
  local numeric_id = getRoomIDbyHash(room_id)
  if not numeric_id or numeric_id < 1 then
    numeric_id = createRoomID()
    addRoom(numeric_id)
    setRoomIDbyHash(numeric_id, room_id)
  end
  setRoomName(numeric_id, name)
  centerview(numeric_id)

  for direction, target_hash in pairs(Tiamud.rooms[room_id].exits) do
    if directions[direction] and safe_string(target_hash, 128) then
      local target_id = getRoomIDbyHash(target_hash)
      if not target_id or target_id < 1 then
        target_id = createRoomID()
        addRoom(target_id)
        setRoomIDbyHash(target_id, target_hash)
      end
      setExit(numeric_id, target_id, direction)
    end
  end
end

local function on_scene()
  if not gmcp or not gmcp.Tiamud or type(gmcp.Tiamud.Scene) ~= "table" then return end
  Tiamud.scene = gmcp.Tiamud.Scene
  raiseEvent("tiamud.scene", Tiamud.scene)
end

local function on_assets()
  if not gmcp or not gmcp.Tiamud or type(gmcp.Tiamud.Assets) ~= "table" then return end
  if rich_output_blocked() then return end
  local image = gmcp.Tiamud.Assets.image
  if type(image) ~= "table" or not safe_content_addressed_path(image.manifest, false) then return end
  for _, format in ipairs({"avif", "webp", "jpeg"}) do
    if image[format] and not safe_content_addressed_path(image[format], false) then return end
  end
  Tiamud.assets = gmcp.Tiamud.Assets
  raiseEvent("tiamud.assets", Tiamud.assets)
end

local function on_media(action, event)
  if not Tiamud.mediaEnabled or rich_output_blocked() or type(event) ~= "table" then return end
  local safe = {action = action}
  if action == "stop" then
    safe.key = safe_string(event.key or event.name or "all", 128)
    if not safe.key then return end
  else
    safe.url = safe_content_addressed_path(event.url or event.file, true)
    if not safe.url then return end
    safe.key = safe_string(event.key or event.name or safe.url, 128)
    if not safe.key then return end
  end
  -- Deliberately no playback, download, URL-opening, game-command, or code
  -- evaluation API. A profile-local handler may consume this audited event.
  Tiamud.media = safe
  raiseEvent("tiamud.media", Tiamud.media)
end

local function send_settings()
  local blocked = rich_output_blocked()
  local media = Tiamud.mediaEnabled and not blocked and "true" or "false"
  local assets = not blocked and "true" or "false"
  local decorative = Tiamud.decorativeEnabled and "true" or "false"
  local plain = Tiamud.plainMode and "true" or "false"
  local reduced = Tiamud.reducedMotion and "true" or "false"
  local screen_reader = Tiamud.screenReader and "true" or "false"
  local links = blocked and "plain" or Tiamud.linkMode
  local graphics = blocked and "none" or Tiamud.graphicsMode
  sendGMCP('Tiamud.Client.Set {"media":' .. media
      .. ',"assets":' .. assets
      .. ',"links":"' .. links
      .. '","graphics":"' .. graphics
      .. '","plain":' .. plain
      .. ',"reducedMotion":' .. reduced
      .. ',"screenReader":' .. screen_reader
      .. ',"decorative":' .. decorative .. '}')
end

function Tiamud.enableMedia(enabled)
  Tiamud.mediaEnabled = enabled == true
  send_settings()
end

function Tiamud.enableLinks(mode)
  if mode ~= "plain" and mode ~= "mslp" and mode ~= "mxp" then return false end
  Tiamud.linkMode = mode
  send_settings()
  return true
end

function Tiamud.enableGraphics(mode)
  if mode ~= "none" and mode ~= "ansi" and mode ~= "kitty"
      and mode ~= "sixel" and mode ~= "iterm2" then return false end
  Tiamud.graphicsMode = mode
  send_settings()
  return true
end

function Tiamud.setAccessibility(plain, reduced_motion, screen_reader, decorative)
  Tiamud.plainMode = plain == true
  Tiamud.reducedMotion = reduced_motion == true
  Tiamud.screenReader = screen_reader == true
  Tiamud.decorativeEnabled = decorative ~= false
  send_settings()
end

registerAnonymousEventHandler("sysProtocolEnabled", function(_, protocol)
  if protocol ~= "GMCP" then return end
  sendGMCP('Core.Supports.Add ["Room 1","Tiamud.Scene 1","Tiamud.Assets 1","Tiamud.CellFrame 1","Client.Media 1","Char.Vitals 1","Char.Items 1"]')
  send_settings()
end)
registerAnonymousEventHandler("gmcp.Room.Info", on_room_info)
registerAnonymousEventHandler("gmcp.Tiamud.Scene", on_scene)
registerAnonymousEventHandler("gmcp.Tiamud.Assets", on_assets)
registerAnonymousEventHandler("gmcp.Client.Media.Default", function()
  on_media("default", gmcp.Client.Media.Default)
end)
registerAnonymousEventHandler("gmcp.Client.Media.Load", function()
  on_media("load", gmcp.Client.Media.Load)
end)
registerAnonymousEventHandler("gmcp.Client.Media.Play", function()
  on_media("play", gmcp.Client.Media.Play)
end)
registerAnonymousEventHandler("gmcp.Client.Media.Stop", function()
  on_media("stop", gmcp.Client.Media.Stop)
end)
