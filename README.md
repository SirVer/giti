# Installing

On Mac, you might need to install dependencies:

    brew install libssh2 openssl

Download the latest [Release](https://github.com/SirVer/giti/releases) and put
the `g` binary in your `PATH`. Then, inform your shell that `g` is an alias for
`git`.

For `zsh`, put this in your `.zshrc`:

    compdef g='git'

For Bash, put this in your `.bashrc`:

    _completion_loader git
    complete -o bashdefault -o default -o nospace -F _git g

# Updating

Simply run `g --update` to self update the binary to the latest release.

# Building on Mac

    brew install libssh2 openssl
    export PKG_CONFIG_PATH="/usr/local/opt/openssl/lib/pkgconfig"
    cargo build --release

