#!/bin/sh
# Build gram and install it somewhere on your PATH.
#
#   ./install.sh                     ~/.local/bin/gram
#   ./install.sh --prefix /usr/local /usr/local/bin/gram
#   ./install.sh --uninstall         remove it again
#
# The collection is never touched; `gram path` prints where it lives.

set -eu

usage() {
	cat <<'EOF'
usage: install.sh [--prefix DIR] [--uninstall] [--no-build]

  --prefix DIR   install into DIR/bin (default: $PREFIX, else ~/.local)
  --uninstall    remove the installed binary
  --no-build     install the existing target/release/gram as-is
  -h, --help     this message
EOF
}

say() { printf '%s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

prefix=${PREFIX:-}
uninstall=0
build=1

while [ $# -gt 0 ]; do
	case $1 in
	--prefix) [ $# -ge 2 ] || die "--prefix needs a directory"; prefix=$2; shift 2 ;;
	--prefix=*) prefix=${1#--prefix=}; shift ;;
	--uninstall) uninstall=1; shift ;;
	--no-build) build=0; shift ;;
	-h | --help) usage; exit 0 ;;
	*) usage >&2; die "unknown option: $1" ;;
	esac
done

if [ -z "$prefix" ]; then
	if [ "$(id -u)" = 0 ]; then prefix=/usr/local; else prefix=$HOME/.local; fi
fi

bindir=$prefix/bin
dest=$bindir/gram

if [ "$uninstall" = 1 ]; then
	if [ -e "$dest" ]; then
		rm -f "$dest"
		say "removed $dest"
	else
		say "nothing at $dest"
	fi
	exit 0
fi

# Run from the repository root no matter where the script was invoked from.
repo=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$repo"
[ -f Cargo.toml ] || die "no Cargo.toml in $repo"

src=target/release/gram

if [ "$build" = 1 ]; then
	command -v cargo >/dev/null 2>&1 ||
		die "cargo not found — install Rust from https://rustup.rs"
	say "building gram (release)..."
	cargo build --release --locked
fi

[ -x "$src" ] || die "$src not found — drop --no-build to build it"

mkdir -p "$bindir"
[ -w "$bindir" ] || die "$bindir is not writable — try: sudo ./install.sh --prefix $prefix"

# Install to a temp name and rename, so a running gram is never clobbered mid-copy.
tmp=$dest.new.$$
trap 'rm -f "$tmp"' EXIT INT TERM
cp "$src" "$tmp"
chmod 755 "$tmp"
mv -f "$tmp" "$dest"
trap - EXIT INT TERM

say "installed $dest ($("$dest" --version 2>/dev/null || echo gram))"

case ":${PATH}:" in
*":$bindir:"*) ;;
*)
	say ""
	say "$bindir is not on your PATH. Add it:"
	case ${SHELL##*/} in
	zsh) rc=~/.zshrc ;;
	bash) rc=~/.bashrc ;;
	fish) rc="~/.config/fish/config.fish  (fish_add_path $bindir)" ;;
	*) rc="your shell's rc file" ;;
	esac
	say ""
	say "  echo 'export PATH=\"$bindir:\$PATH\"' >> $rc"
	say "  exec \$SHELL"
	;;
esac

say ""
say "Then, from anywhere:"
say "  gram import $repo/decks/word_smart.csv \"Word Smart\""
say "  gram"
