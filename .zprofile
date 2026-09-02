# homebrew shellenv
export HOMEBREW_PREFIX="/opt/homebrew"
export HOMEBREW_CELLAR="/opt/homebrew/Cellar"
export HOMEBREW_REPOSITORY="/opt/homebrew"
export PATH="/opt/homebrew/bin:/opt/homebrew/sbin${PATH+:$PATH}"
export MANPATH="/opt/homebrew/share/man${MANPATH+:$MANPATH}:"
export INFOPATH="/opt/homebrew/share/info:${INFOPATH:-}"

# ruby (before compile flags — GEM_BINDIR used below)
export GEM_BINDIR="$(ruby -e 'puts Gem.bindir')"

# compile flags
export HOMEBREW_OPT="$HOMEBREW_PREFIX/opt"
export PATH="$HOMEBREW_OPT/gettext/bin:$HOMEBREW_OPT/curl/bin:$HOMEBREW_OPT/openssl/bin:$HOMEBREW_OPT/sqlite/bin:$HOMEBREW_OPT/ncurses/bin:$HOMEBREW_OPT/ruby/bin:$GEM_BINDIR:$PATH"
export CPPFLAGS="-I$HOMEBREW_OPT/gettext/include -I$HOMEBREW_OPT/curl/include -I$HOMEBREW_OPT/openssl/include -I$HOMEBREW_OPT/readline/include -I$HOMEBREW_OPT/sqlite/include -I$HOMEBREW_OPT/ncurses/include -I$HOMEBREW_OPT/ruby/include $CPPFLAGS"
export LDFLAGS="-L$HOMEBREW_OPT/gettext/lib -L$HOMEBREW_OPT/curl/lib -L$HOMEBREW_OPT/openssl/lib -L$HOMEBREW_OPT/readline/lib -L$HOMEBREW_OPT/sqlite/lib -L$HOMEBREW_OPT/ncurses/lib -L$HOMEBREW_OPT/ruby/lib $LDFLAGS"

# language runtimes

## python
# managed by uv — python/python3/pip/pip3 shims live in ~/.local/bin

## go
export GOPATH="$HOME/go"
export PATH="$GOPATH/bin:$PATH"
export GOCACHE=/tmp
export GOSUMDB=off

## node
# managed by fnm — eval "$(fnm env)" in .zshrc

# cloud
export KUBECONFIG=~/.kube/config

# misc tools
export PATH="$HOME/.local/bin:$PATH"
export BUN_INSTALL="$HOME/.bun"
export PATH="$BUN_INSTALL/bin:$PATH"
