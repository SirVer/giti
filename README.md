
# Building on Mac

    brew install libssh2 openssl
    export PKG_CONFIG_PATH="/usr/local/opt/openssl/lib/pkgconfig"
    cargo build --release
