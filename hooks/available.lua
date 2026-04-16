local http = require("http")
local json = require("json")

local REPO = "gracefullight/mise-monodep-plugin"

function PLUGIN:Available(ctx)
    local url = "https://api.github.com/repos/" .. REPO .. "/releases"
    local resp, err = http.get({ url = url, headers = { Accept = "application/vnd.github.v3+json" } })

    if err ~= nil or resp.status_code ~= 200 then
        -- Fallback to hardcoded version if API fails
        return {
            { version = "0.1.0" }
        }
    end

    local releases = json.decode(resp.body)
    local versions = {}

    for _, release in ipairs(releases) do
        if not release.prerelease and not release.draft then
            local tag = release.tag_name
            -- Strip leading "v" if present
            local version = tag:match("^v?(.+)$")
            if version ~= nil then
                versions[#versions + 1] = { version = version }
            end
        end
    end

    if #versions == 0 then
        return {
            { version = "0.1.0" }
        }
    end

    return versions
end
