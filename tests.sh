#!/usr/bin/env bash
# Tests de claude-accounts : HOME temporaire, faux claude, faux gum.
# Aucun vrai compte, aucun identifiant, aucun réseau.

set -uo pipefail

SCRIPT=$(readlink -f -- "$(dirname -- "$0")/claude-accounts")
TESTS=0
FAILURES=0

check() {
	local label="$1"
	shift
	TESTS=$((TESTS + 1))
	if "$@"; then
		printf 'ok   %s\n' "$label"
	else
		FAILURES=$((FAILURES + 1))
		printf 'FAIL %s\n' "$label"
	fi
}

setup() {
	SANDBOX=$(mktemp -d)
	export HOME="$SANDBOX/home"
	unset XDG_CONFIG_HOME XDG_DATA_HOME TERMINAL
	mkdir -p "$HOME/.claude/skills" "$SANDBOX/bin" "$SANDBOX/nobin"
	printf '# commun\n' >"$HOME/.claude/CLAUDE.md"
	printf '{}\n' >"$HOME/.claude/settings.json"
	cat >"$SANDBOX/bin/claude" <<'EOF'
#!/usr/bin/env bash
if [[ "${1:-} ${2:-}" == 'auth status' ]]; then
	[[ -f "$CLAUDE_CONFIG_DIR/.fake-broken" ]] && exit 2
	if [[ -n "${ANTHROPIC_API_KEY:-}" ]]; then
		printf '{\n  "loggedIn": true,\n  "authMethod": "api_key"\n}\n'
		exit 0
	fi
	if [[ -f "$CLAUDE_CONFIG_DIR/.fake-plan" ]]; then
		printf '{\n  "loggedIn": true,\n  "subscriptionType": "%s"\n}\n' \
			"$(<"$CLAUDE_CONFIG_DIR/.fake-plan")"
		exit 0
	fi
	printf '{\n  "loggedIn": false\n}\n'
	exit 1
fi
printf 'CONFIG=%s\n' "$CLAUDE_CONFIG_DIR"
printf 'APIKEY=%s\n' "${ANTHROPIC_API_KEY:-unset}"
printf 'ARGS=%s\n' "$*"
EOF
	mkdir -p "$SANDBOX/gum"
	cat >"$SANDBOX/gum/gum" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"$SANDBOX/gum.log"
case "\$1" in
choose) [[ -n "\${FAKE_CHOICE:-}" ]] || exit 130; printf '%s\n' "\$FAKE_CHOICE" ;;
input) printf '%s\n' "\${FAKE_INPUT:-}" ;;
esac
EOF
	chmod +x "$SANDBOX/bin/claude" "$SANDBOX/gum/gum"
	BASE_PATH="/usr/bin:/bin"
	export PATH="$SANDBOX/bin:$BASE_PATH"
}

ca() {
	"$SCRIPT" "$@"
}

data_dir() {
	printf '%s\n' "$HOME/.local/share/claude-accounts/$1"
}

perm_of() {
	stat -c '%a' -- "$1"
}

with_tty() {
	local input="$1"
	shift
	printf '%s\n' "$input" | script -qec "$*" /dev/null
}

test_add_and_list() {
	ca add pro1 >/dev/null
	ca add pro2 >/dev/null
	check "add crée le dossier en 700" test "$(perm_of "$(data_dir pro1)")" = 700
	check "dossier racine en 700" test "$(perm_of "$HOME/.local/share/claude-accounts")" = 700
	check "list affiche les deux comptes" test "$(ca list)" = $'pro1\npro2'
	check "add refuse un doublon" bash -c '! "$0" add pro1 2>/dev/null' "$SCRIPT"
	check "add refuse un nom dangereux" bash -c '! "$0" add ../x 2>/dev/null' "$SCRIPT"
	check "CLAUDE.md partagé par lien" test "$(readlink "$(data_dir pro1)/CLAUDE.md")" = "$HOME/.claude/CLAUDE.md"
	check "skills partagé par lien" test -L "$(data_dir pro1)/skills"
	check "settings.json non lié" test ! -e "$(data_dir pro1)/settings.json"
}

test_run() {
	local out
	out=$(ANTHROPIC_API_KEY=secret ca run pro2 --resume)
	check "run passe le bon CLAUDE_CONFIG_DIR" grep -qxF "CONFIG=$(data_dir pro2)" <<<"$out"
	check "run retire ANTHROPIC_API_KEY" grep -qx 'APIKEY=unset' <<<"$out"
	check "run charge settings.json commun et passe les arguments" \
		grep -qxF "ARGS=--settings $HOME/.claude/settings.json --resume" <<<"$out"
	check "deux comptes, deux dossiers distincts" \
		test "$(ca run pro1 | sed -n '1p')" != "$(ca run pro2 | sed -n '1p')"
	mkdir -p "$HOME/.claude/agents"
	ca run pro1 >/dev/null
	check "run lie un dossier partagé apparu après add" test -L "$(data_dir pro1)/agents"
}

test_status() {
	local out
	printf 'pro\n' >"$(data_dir pro1)/.fake-plan"
	out=$(ca status)
	check "status : compte connecté avec son offre" grep -qE '^pro1 +logged in +pro$' <<<"$out"
	check "status : compte neuf déconnecté" grep -qE '^pro2 +logged out$' <<<"$out"
	out=$(ANTHROPIC_API_KEY=secret ca status)
	check "status retire ANTHROPIC_API_KEY" grep -qE '^pro2 +logged out$' <<<"$out"
	touch "$(data_dir pro2)/.fake-broken"
	out=$(ca status)
	check "status : erreur de claude = unknown" grep -qE '^pro2 +unknown$' <<<"$out"
	rm -- "$(data_dir pro2)/.fake-broken"
	check "status sans claude échoue" bash -c \
		'PATH=$1 "$0" status 2>&1 | grep -q "claude not found"' "$SCRIPT" "$SANDBOX/nobin:$BASE_PATH"
}

test_sessions() {
	local out live other sessions
	sessions="$(data_dir pro1)/sessions"
	mkdir -p "$sessions" "$SANDBOX/proc"
	cp "$(command -v sleep)" "$SANDBOX/proc/claude"
	"$SANDBOX/proc/claude" 30 &
	live=$!
	sleep 30 &
	other=$!
	touch "$sessions/$live.json" "$sessions/$other.json" "$sessions/999999999.json"
	out=$(ca status)
	check "status compte seulement les sessions claude vivantes" \
		grep -qE '^pro1 +logged in +pro +1 active$' <<<"$out"
	check "status : aucun compteur sans session" grep -qE '^pro2 +logged out$' <<<"$out"
	PATH="$SANDBOX/gum:$SANDBOX/bin:$BASE_PATH" FAKE_CHOICE=Quit with_tty '' "$SCRIPT" >/dev/null
	check "menu gum : sessions actives affichées" \
		grep -qF 'pro1  logged in · pro · 1 active' "$SANDBOX/gum.log"
	kill "$live" "$other"
	wait "$live" "$other" 2>/dev/null
	rm -rf -- "$sessions"
}

json_true() {
	jq -e "$1" "$2" >/dev/null
}

test_shared_mcp() {
	local out shared="$HOME/.local/share/claude-accounts/.shared-mcp"
	printf '{"mcpServers":{"alpha":{"type":"stdio","command":"a"},"both":{"type":"stdio","command":"other"}}}\n' \
		>"$(data_dir pro2)/.claude.json"
	printf '{"numStartups":1,"mcpServers":{"both":{"type":"stdio","command":"own"}},"projects":{"/p":{"mcpServers":{"proj":{"type":"stdio","command":"own"}}}}}\n' \
		>"$(data_dir pro1)/.claude.json"
	ca add pro4 >/dev/null
	printf '{"mcpServers":{"alpha":{"type":"stdio","command":"z"},"proj":{"type":"stdio","command":"z"}}}\n' \
		>"$(data_dir pro4)/.claude.json"
	out=$(ca run pro1 hello)
	check "run passe les MCP des autres comptes avec --mcp-config=" grep -qxF \
		"ARGS=--settings $HOME/.claude/settings.json --mcp-config=$shared/pro1.json hello" <<<"$out"
	check "MCP d'un autre compte partagé, le premier compte gagne" \
		json_true '.mcpServers.alpha.command == "a"' "$shared/pro1.json"
	check "MCP déjà présent dans le compte lancé non transmis" \
		json_true '.mcpServers | has("both") | not' "$shared/pro1.json"
	check "MCP lié à un projet du compte lancé non transmis" \
		json_true '.mcpServers | has("proj") | not' "$shared/pro1.json"
	check "fichier commun en 700" test "$(perm_of "$shared")" = 700
	printf '{"mcpServers": {' >"$(data_dir pro2)/.claude.json"
	out=$(ca run pro1 2>&1)
	check "compte au JSON cassé ignoré avec avertissement" grep -q 'pro2 skipped' <<<"$out"
	check "compte au JSON cassé : les autres restent partagés" \
		json_true '.mcpServers.alpha.command == "z"' "$shared/pro1.json"
	printf '{"mcpServers": {' >"$(data_dir pro1)/.claude.json"
	out=$(ca run pro1)
	check "JSON du compte lancé cassé : claude lancé sans --mcp-config" bash -c \
		'grep -q "^CONFIG=" <<<"$0" && ! grep -q -- --mcp-config <<<"$0"' "$out"
	printf '{}\n' >"$(data_dir pro1)/.claude.json"
	out=$(PATH="$SANDBOX/claude-only:$SANDBOX/sys" "$SCRIPT" run pro1)
	check "sans jq : claude lancé sans --mcp-config" bash -c \
		'grep -q "^CONFIG=" <<<"$0" && ! grep -q -- --mcp-config <<<"$0"' "$out"
	ca remove pro4 </dev/null >/dev/null
	rm -f -- "$(data_dir pro1)/.claude.json" "$(data_dir pro2)/.claude.json"
	out=$(ca run pro1)
	check "aucun MCP chez les autres : pas de --mcp-config" bash -c '! grep -q -- --mcp-config <<<"$0"' "$out"
}

test_errors() {
	check "run sur compte inexistant échoue" bash -c '! "$0" run nope 2>/dev/null' "$SCRIPT"
	check "message compte inexistant" bash -c '"$0" run nope 2>&1 | grep -q "no such account"' "$SCRIPT"
	check "claude absent du PATH" bash -c \
		'PATH=$1 "$0" run pro1 2>&1 | grep -q "claude not found"' "$SCRIPT" "$SANDBOX/nobin:$BASE_PATH"
	check "commande inconnue échoue" bash -c '! "$0" foo 2>/dev/null' "$SCRIPT"
	check "add sans nom échoue" bash -c '! "$0" add 2>/dev/null' "$SCRIPT"
}

test_version() {
	local pattern='^claude-accounts [0-9]+\.[0-9]+\.[0-9]+$'
	check "--version affiche nom et version x.y.z" bash -c '[[ $("$0" --version) =~ $1 ]]' "$SCRIPT" "$pattern"
	check "-V équivaut à --version" bash -c '[[ $("$0" -V) == $("$0" --version) ]]' "$SCRIPT"
	check "--help mentionne --version" bash -c '"$0" --help | grep -q -- "--version"' "$SCRIPT"
}

test_menu_gum() {
	local out gum_path="$SANDBOX/gum:$SANDBOX/bin:$BASE_PATH"
	out=$(PATH="$gum_path" FAKE_CHOICE=pro2 with_tty '' "$SCRIPT")
	check "menu gum lance claude ici avec le compte choisi" \
		grep -qF "CONFIG=$(data_dir pro2)" <<<"$out"
	check "menu gum présente les comptes et les actions de gestion" bash -c \
		'grep -q -- "--height 12" "$0" && grep -qF "+ Add an account" "$0" && grep -qF -- "- Remove an account" "$0"' \
		"$SANDBOX/gum.log"
	check "menu gum : compte connecté affiché avec son offre" grep -qF 'pro1  logged in · pro' "$SANDBOX/gum.log"
	check "menu gum : compte déconnecté signalé" grep -qF 'pro2  logged out' "$SANDBOX/gum.log"
	out=$(PATH="$gum_path" FAKE_CHOICE='pro1  logged in · pro' with_tty '' "$SCRIPT")
	check "menu gum : choisir un libellé lance le bon compte" \
		grep -qF "CONFIG=$(data_dir pro1)" <<<"$out"
	out=$(PATH="$gum_path" with_tty '' "$SCRIPT"; printf 'EXIT=%s' "$?")
	check "menu gum annulé (Échap) : rien lancé, sortie 0" \
		bash -c '! grep -q CONFIG= <<<"$0" && grep -q EXIT=0 <<<"$0"' "$out"
	out=$(PATH="$gum_path" FAKE_CHOICE=Quit with_tty '' "$SCRIPT")
	check "menu gum Quitter : rien lancé" bash -c '! grep -q CONFIG= <<<"$0"' "$out"
	out=$(PATH="$gum_path" FAKE_CHOICE='+ Add an account' FAKE_INPUT=pro9 with_tty '' "$SCRIPT")
	check "menu gum ajout : compte créé" test -d "$(data_dir pro9)"
	check "menu gum ajout : claude lancé sur le nouveau compte" \
		grep -qF "CONFIG=$(data_dir pro9)" <<<"$out"
	ca remove pro9 </dev/null >/dev/null
}

test_menu_select() {
	local out
	out=$(PATH="$SANDBOX/claude-only:$SANDBOX/sys" with_tty 1 "$SCRIPT" 2>&1)
	check "menu sans gum : select lance le compte 1 ici" \
		grep -qF "CONFIG=$(data_dir pro1)" <<<"$out"
}

test_remove() {
	ca remove pro2 </dev/null >/dev/null
	check "remove retire de la liste" test "$(ca list)" = pro1
	check "remove sans confirmation garde le dossier" test -d "$(data_dir pro2)"
	ca add pro3 >/dev/null
	PATH="$SANDBOX/gum:$SANDBOX/bin:$BASE_PATH" with_tty o "$SCRIPT" remove pro3 >/dev/null
	check "remove confirmé supprime le dossier" test ! -e "$(data_dir pro3)"
	check "remove sur compte inexistant échoue" bash -c '! "$0" remove nope 2>/dev/null' "$SCRIPT"
}

make_minimal_path() {
	local tool
	mkdir -p "$SANDBOX/claude-only" "$SANDBOX/sys"
	cp "$SANDBOX/bin/claude" "$SANDBOX/claude-only/claude"
	for tool in bash env grep mkdir chmod ln mktemp mv rm ps readlink basename dirname setsid script; do
		ln -s "$(command -v "$tool")" "$SANDBOX/sys/$tool"
	done
}

setup
make_minimal_path
test_add_and_list
test_run
test_status
test_sessions
test_shared_mcp
test_errors
test_version
test_menu_gum
test_menu_select
test_remove
rm -rf -- "$SANDBOX"
printf '\n%d tests, %d échecs\n' "$TESTS" "$FAILURES"
((FAILURES == 0))
