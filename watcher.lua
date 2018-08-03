-- A watcher file for shell_grunt2: https://github.com/SirVer/shell_grunt2

function is_rust(p) 
   if p:find("target") ~= nil then return false end
   return p:ext() == "rs" or p:ext() == "toml"
end

return {
  {
    environment = {
       CARGO_INCREMENTAL = "1",
       PKG_CONFIG_PATH="/usr/local/opt/openssl/lib/pkgconfig",
    },
    commands = {
      -- {
        -- name = "Cargo check",
        -- command = "cargo check --color=always",
      -- },
      {
        name = "Cargo build [release]",
        command = "cargo build --release --color=always",
      },
    },
    should_run = is_rust,
    redirect_stderr = "/tmp/cargo.err",
    start_delay = 50,
  },
  {
    commands = {
      {
        name = "Rusty tags.",
        command = "rusty-tags vi",
      },
    },
    should_run = is_rust,
    redirect_stderr = "/tmp/cargo.err",
    start_delay = 50,
  },
  -- {
    -- name = "Cargo clippy",
    -- command = "rustup run nightly cargo clippy --color=always",
    -- should_run = function(p)
      -- if p:find("target") ~=tnil then return false end
      -- return p:ext() == "rs"
    -- end,
    -- work_directory = "sdl_viewer",
    -- redirect_stderr = "/tmp/cargo.err",
    -- start_delay = 50,
  -- },
}
