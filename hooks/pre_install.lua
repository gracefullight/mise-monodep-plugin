local REPO = "gracefullight/mise-monodep-plugin"

local function platform()
    local os = RUNTIME.osType
    local arch = RUNTIME.archType
    -- Map mise runtime values to release artifact names
    local os_map = { darwin = "apple-darwin", linux = "unknown-linux-gnu" }
    local arch_map = { amd64 = "x86_64", arm64 = "aarch64" }
    return (arch_map[arch] or arch) .. "-" .. (os_map[os] or os)
end

function PLUGIN:PreInstall(ctx)
    local version = ctx.version
    local target = platform()
    local filename = "monodep-" .. target .. ".tar.gz"
    local url = "https://github.com/" .. REPO .. "/releases/download/v" .. version .. "/" .. filename

    return {
        version = version,
        url = url,
    }
end
