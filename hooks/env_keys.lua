function PLUGIN:EnvKeys(ctx)
    local mainPath = ctx.path
    return {
        {
            key = "MONODEP_HOME",
            value = mainPath
        },
        {
            key = "PATH",
            value = mainPath .. "/bin"
        }
    }
end
