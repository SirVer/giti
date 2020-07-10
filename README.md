# Installing

On Mac, you need to install dependencies:

    brew install libssh2 openssl@1.1

Download the latest [Release](https://github.com/SirVer/giti/releases) and put
the `g` binary in your `PATH`. Then, inform your shell that `g` is an alias for
`git`.

For `zsh`, put this in your `.zshrc`:

    compdef g='git'

For Bash, put this in your `.bashrc`:

    _completion_loader git
    complete -o bashdefault -o default -o nospace -F _git g

On Linux, you might need to set `SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt` for the tool to work properly.
This is due to differences where SSL certs are saved in Debian and Ubuntu.

# Running fix commands

When running `g fix`, the tool will figure out which files have changed compared
to `origin/master` and run auto-formatting tool on them. You need to install
them separately and have them on your `$PATH`:

- **clang-format:** For c++ files and protobuf. Install with `brew install clang-format`.
- **rustfmt:** For rust files. Install with `rustup run cargo install --force rustfmt`.
- **buildifier:** For bazel BUILD files. Install go, make sure `$GOPATH/bin` is
  in your `$PATH`, then

      go get github.com/bazelbuild/buildtools/buildifier
      go install github.com/bazelbuild/buildtools/buildifier

# Updating

Simply run `g --update` to self update the binary to the latest release.

# Building on Mac

    brew install libssh2 openssl@1.1
    export PKG_CONFIG_PATH="/usr/local/opt/openssl/lib/pkgconfig"
    cargo build --release

