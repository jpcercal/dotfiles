# homebrew shellenv
export HOMEBREW_PREFIX="/opt/homebrew"
export HOMEBREW_CELLAR="/opt/homebrew/Cellar"
export HOMEBREW_REPOSITORY="/opt/homebrew"
export PATH="/opt/homebrew/bin:/opt/homebrew/sbin${PATH+:$PATH}"
export MANPATH="/opt/homebrew/share/man${MANPATH+:$MANPATH}:"
export INFOPATH="/opt/homebrew/share/info:${INFOPATH:-}"

# ruby (derive the gem bindir from the installed brew ruby, no subprocess forks)
gemdirs=("$HOMEBREW_PREFIX"/lib/ruby/gems/*(/N))
if [[ -n ${gemdirs[-1]} ]]; then
    export GEM_BINDIR="${gemdirs[-1]}/bin"
fi
unset gemdirs

# compile flags
export HOMEBREW_OPT="$HOMEBREW_PREFIX/opt"

# gettext
export PATH="$HOMEBREW_OPT/gettext/bin:$PATH"
export CPPFLAGS="-I$HOMEBREW_OPT/gettext/include $CPPFLAGS"
export LDFLAGS="-L$HOMEBREW_OPT/gettext/lib $LDFLAGS"

# curl
export PATH="$HOMEBREW_OPT/curl/bin:$PATH"
export CPPFLAGS="-I$HOMEBREW_OPT/curl/include $CPPFLAGS"
export LDFLAGS="-L$HOMEBREW_OPT/curl/lib $LDFLAGS"

# openssl
export PATH="$HOMEBREW_OPT/openssl/bin:$PATH"
export CPPFLAGS="-I$HOMEBREW_OPT/openssl/include $CPPFLAGS"
export LDFLAGS="-L$HOMEBREW_OPT/openssl/lib $LDFLAGS"

# readline
export CPPFLAGS="-I$HOMEBREW_OPT/readline/include $CPPFLAGS"
export LDFLAGS="-L$HOMEBREW_OPT/readline/lib $LDFLAGS"

# sqlite
export PATH="$HOMEBREW_OPT/sqlite/bin:$PATH"
export CPPFLAGS="-I$HOMEBREW_OPT/sqlite/include $CPPFLAGS"
export LDFLAGS="-L$HOMEBREW_OPT/sqlite/lib $LDFLAGS"

# ncurses
export PATH="$HOMEBREW_OPT/ncurses/bin:$PATH"
export CPPFLAGS="-I$HOMEBREW_OPT/ncurses/include $CPPFLAGS"
export LDFLAGS="-L$HOMEBREW_OPT/ncurses/lib $LDFLAGS"

# ruby
export PATH="$HOMEBREW_OPT/ruby/bin:$GEM_BINDIR:$PATH"
export CPPFLAGS="-I$HOMEBREW_OPT/ruby/include $CPPFLAGS"
export LDFLAGS="-L$HOMEBREW_OPT/ruby/lib $LDFLAGS"

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

# editor
export EDITOR=nvim
export VISUAL=nvim

# misc tools
export PATH="$HOME/.local/bin:$PATH"
export PATH="$HOME/.opencode/bin:$PATH"
export BUN_INSTALL="$HOME/.bun"
export PATH="$BUN_INSTALL/bin:$PATH"
