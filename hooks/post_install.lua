local fs = require("lib/fs")

function PLUGIN:PostInstall(ctx)
    local sdkInfo = ctx.sdkInfo["monodep"]
    local installPath = sdkInfo.path
    local binSource = installPath .. "/monodep"
    local binTarget = installPath .. "/bin/monodep"

    -- Move extracted binary to bin/
    if fs.exists(binSource) then
        fs.ensure_dir(installPath .. "/bin")
        os.rename(binSource, binTarget)
        fs.make_executable(binTarget)
    end

    -- Ensure .monodep/ is in root .gitignore
    local cwd = os.getenv("PWD") or "."
    local gitignorePath = cwd .. "/.gitignore"

    local entries = {
        { pattern = "^%.monodep/?$",    line = ".monodep/",     comment = "# monodep store (hardlink dedup)" },
        { pattern = "^node_modules/?$", line = "node_modules/", comment = "# dependencies" },
        { pattern = "^%.venv/?$",       line = ".venv/",        comment = nil },
    }

    local existing = {}
    local file = io.open(gitignorePath, "r")
    if file ~= nil then
        for line in file:lines() do
            existing[#existing + 1] = line
        end
        file:close()
    end

    local missing = {}
    for _, entry in ipairs(entries) do
        local found = false
        for _, line in ipairs(existing) do
            if line:match(entry.pattern) then
                found = true
                break
            end
        end
        if not found then
            missing[#missing + 1] = entry
        end
    end

    if #missing > 0 then
        local append = io.open(gitignorePath, "a")
        if append ~= nil then
            local lastComment = nil
            for _, entry in ipairs(missing) do
                if entry.comment ~= nil and entry.comment ~= lastComment then
                    append:write("\n" .. entry.comment .. "\n")
                    lastComment = entry.comment
                end
                append:write(entry.line .. "\n")
            end
            append:close()
        end
    end
end
