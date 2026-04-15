--- scopeon.lua — Neovim plugin for Scopeon AI observability
---
--- Shows live AI cost and cache hit rate in your statusline.
--- Requires Neovim 0.8+ and scopeon running with `scopeon start`.
---
--- Quick start (lazy.nvim):
---   { "scopeon/scopeon", config = function() require("scopeon").setup() end }
---
--- Manual setup — add to your statusline:
---   require("scopeon").setup()
---   -- Then use require("scopeon").status() in your lualine/galaxyline config.

local M = {}

local _status = ""
local _timer = nil
local PORT = 7771

--- Fetch stats from the local Scopeon HTTP API.
local function fetch()
    local ok, result = pcall(function()
        local handle = io.popen(
            string.format(
                "curl -sf --connect-timeout 1 http://127.0.0.1:%d/api/v1/stats 2>/dev/null",
                PORT
            )
        )
        if not handle then return nil end
        local raw = handle:read("*a")
        handle:close()
        return raw
    end)
    if not ok or not result or result == "" then
        _status = ""
        return
    end
    local ok2, data = pcall(vim.json.decode, result)
    if not ok2 or type(data) ~= "table" then
        _status = ""
        return
    end
    local cost = data.today_cost_usd or 0
    local cache = (data.cache_hit_rate or 0) * 100
    local health = data.health_score or 0
    _status = string.format("⬡%d  %.0f%%  $%.2f", health, cache, cost)
end

--- Return the current status string (for use in statusline).
function M.status()
    return _status
end

--- Configure the port (default: 7771).
function M.set_port(port)
    PORT = port
end

--- Start the background polling loop (call once in your init).
function M.setup(opts)
    opts = opts or {}
    if opts.port then PORT = opts.port end
    fetch()
    if _timer then _timer:stop() end
    _timer = vim.loop.new_timer()
    _timer:start(0, 30000, vim.schedule_wrap(fetch))
end

return M
