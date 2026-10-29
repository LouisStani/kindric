#!/usr/bin/env bash
# Tests de claude-accounts : HOME temporaire, faux claude.
# Aucun vrai compte, aucun identifiant, aucun réseau.

set -uo pipefail
# A permissive umask, so that only the tool itself can make its files private.
umask 022

SCRIPT=$(readlink -f -- "${CA_BIN:-$(dirname -- "$0")/target/release/claude-accounts}")
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
	chmod +x "$SANDBOX/bin/claude"
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

# Types raw keys into the program, in a pseudo-terminal (no newline added).
with_keys() {
	local keys="$1"
	shift
	printf '%s' "$keys" | script -qec "$*" /dev/null
}

# Menu screen as text: opens the menu and quits it.
menu_screen() {
	with_keys $'quit\r' "$SCRIPT"
}

with_tty() {
	local input="$1"
	shift
	printf '%s\n' "$input" | script -qec "$*" /dev/null
}

test_list_empty() {
	check "list sans compte : stdout vide" test -z "$(ca list 2>/dev/null)"
	check "list sans compte : indication sur stderr" bash -c \
		'"$0" list 2>&1 >/dev/null | grep -q "No accounts"' "$SCRIPT"
	check "list --json sans compte : []" test "$(ca list --json)" = '[]'
	check "status --json sans compte : [] même sans claude" test \
		"$(PATH="$SANDBOX/nobin:$BASE_PATH" ca status --json)" = '[]'
}

test_add_and_list() {
	ca add pro1 >/dev/null
	ca add pro2 >/dev/null
	check "list --json : tableau JSON des noms" bash -c \
		'"$0" list --json | jq -e ". == [\"pro1\",\"pro2\"]" >/dev/null' "$SCRIPT"
	check "list refuse une option inconnue" bash -c '! "$0" list --tsv 2>/dev/null' "$SCRIPT"
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
	check "status --json : erreur de claude = unknown" bash -c \
		'"$0" status --json | jq -e ".[1].state == \"unknown\"" >/dev/null' "$SCRIPT"
	rm -- "$(data_dir pro2)/.fake-broken"
	check "status sans claude échoue" bash -c \
		'PATH=$1 "$0" status 2>&1 | grep -q "claude not found"' "$SCRIPT" "$SANDBOX/nobin:$BASE_PATH"
	check "status --json sans claude échoue" bash -c \
		'! PATH=$1 "$0" status --json >/dev/null 2>&1' "$SCRIPT" "$SANDBOX/nobin:$BASE_PATH"
	out=$(ca status --json)
	check "status --json : compte connecté complet" jq -e --arg d "$(data_dir pro1)" \
		'.[0] == {name: "pro1", state: "logged-in", sessions: 0, plan: "pro", dir: $d, isolated: false}' <<<"$out"
	check "status --json : compte déconnecté, offre null" jq -e --arg d "$(data_dir pro2)" \
		'.[1] == {name: "pro2", state: "logged-out", sessions: 0, plan: null, dir: $d, isolated: false}' <<<"$out"
	check "status refuse une option inconnue" bash -c '! "$0" status --tsv 2>/dev/null' "$SCRIPT"
}

test_status_json_escaping() {
	local weird="$SANDBOX/a\"b\\c"$'\t'"d" out
	XDG_DATA_HOME="$weird" XDG_CONFIG_HOME="$SANDBOX/cfg-weird" ca add esc >/dev/null
	out=$(XDG_DATA_HOME="$weird" XDG_CONFIG_HOME="$SANDBOX/cfg-weird" ca status --json)
	check "status --json échappe \" \\ et tabulation dans dir" \
		jq -e --arg d "$weird/claude-accounts/esc" '.[0].dir == $d' <<<"$out"
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
	check "status --json : sessions est un nombre" bash -c \
		'"$0" status --json | jq -e ".[0].sessions == 1" >/dev/null' "$SCRIPT"
	check "status : aucun compteur sans session" grep -qE '^pro2 +logged out$' <<<"$out"
	check "menu : sessions actives affichées" grep -qF 'pro1  logged in · pro · 1 active' <<<"$(menu_screen)"
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
	check "sans jq : MCP partagés quand même (plus besoin de jq)" \
		grep -qF -- "--mcp-config=$shared/pro1.json" <<<"$out"
	ca remove pro4 </dev/null >/dev/null
	rm -f -- "$(data_dir pro1)/.claude.json" "$(data_dir pro2)/.claude.json"
	out=$(ca run pro1)
	check "aucun MCP chez les autres : pas de --mcp-config" bash -c '! grep -q -- --mcp-config <<<"$0"' "$out"
}

test_isolation() {
	local out shared="$HOME/.local/share/claude-accounts/.shared-mcp"
	ca add work >/dev/null
	printf '{"mcpServers":{"secret":{"type":"stdio","command":"w"}}}\n' >"$(data_dir work)/.claude.json"
	printf '{"mcpServers":{"perso":{"type":"stdio","command":"p"}}}\n' >"$(data_dir pro2)/.claude.json"
	ca run pro1 >/dev/null
	check "avant isolement : MCP de work partagé" json_true '.mcpServers | has("secret")' "$shared/pro1.json"
	check "isolate sur compte inexistant échoue" bash -c '! "$0" isolate nope 2>/dev/null' "$SCRIPT"
	ca isolate work >/dev/null
	check "isolate deux fois : pas de doublon" bash -c \
		'"$0" isolate work >/dev/null && test "$("$0" isolated)" = work' "$SCRIPT"
	check "isolated --json" test "$(ca isolated --json)" = '["work"]'
	check "liste des isolés en 600" test "$(perm_of "$HOME/.config/claude-accounts/isolated")" = 600
	ca run pro1 >/dev/null
	check "MCP d'un compte isolé jamais donné" json_true '.mcpServers | has("secret") | not' "$shared/pro1.json"
	check "MCP des autres comptes toujours partagés" json_true '.mcpServers | has("perso")' "$shared/pro1.json"
	out=$(ca run work)
	check "compte isolé lancé avec un fichier MCP" grep -qF -- "--mcp-config=$shared/work.json" <<<"$out"
	check "compte isolé ne reçoit aucun MCP" json_true '.mcpServers == {}' "$shared/work.json"
	rm -f -- "$(data_dir pro2)/.claude.json"
	out=$(ca run pro1)
	check "plus rien à partager : ancien fichier vidé, pas réutilisé" bash -c \
		'jq -e ".mcpServers == {}" "$1" >/dev/null && ! grep -q -- --mcp-config <<<"$0"' "$out" "$shared/pro1.json"
	check "status --json : champ isolated" bash -c \
		'"$0" status --json | jq -e "map(select(.isolated) | .name) == [\"work\"]" >/dev/null' "$SCRIPT"
	check "status texte : isolated" bash -c '"$0" status | grep -qE "^work .*  isolated$"' "$SCRIPT"
	check "menu : compte isolé signalé" grep -qF 'work  logged out · isolated' <<<"$(menu_screen)"
	ca unisolate work >/dev/null
	check "unisolate : plus isolé" test -z "$(ca isolated)"
	check "unisolate sur compte non isolé : sans effet" bash -c '"$0" unisolate work >/dev/null' "$SCRIPT"
	ca isolate work >/dev/null
	ca remove work --delete-dir >/dev/null
	check "remove retire aussi de la liste des isolés" test -z "$(ca isolated)"
	rm -f -- "$(data_dir pro1)/.claude.json"
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

test_menu_keys() {
	local out
	out=$(with_keys $'pro2\r' "$SCRIPT")
	check "menu : lance claude ici avec le compte choisi" grep -qF "CONFIG=$(data_dir pro2)" <<<"$out"
	check "menu : présente les comptes et les actions de gestion" bash -c \
		'grep -qF "+ Add an account" <<<"$0" && grep -qF -- "- Remove an account" <<<"$0"' "$out"
	check "menu : compte connecté affiché avec son offre" grep -qF 'pro1  logged in · pro' <<<"$out"
	check "menu : compte déconnecté signalé" grep -qF 'pro2  logged out' <<<"$out"
	out=$(with_keys $'\x1b' "$SCRIPT"; printf 'EXIT=%s' "$?")
	check "menu annulé (Échap) : rien lancé, sortie 0" \
		bash -c '! grep -q CONFIG= <<<"$0" && grep -q EXIT=0 <<<"$0"' "$out"
	out=$(with_keys $'quit\r' "$SCRIPT")
	check "menu Quit : rien lancé" bash -c '! grep -q CONFIG= <<<"$0"' "$out"
	out=$(with_keys $'add\rpro9\r' "$SCRIPT")
	check "menu ajout : compte créé" test -d "$(data_dir pro9)"
	check "menu ajout : claude lancé sur le nouveau compte" grep -qF "CONFIG=$(data_dir pro9)" <<<"$out"
	ca remove pro9 --delete-dir >/dev/null
	out=$(PATH="$SANDBOX/claude-only:$SANDBOX/sys" with_keys $'pro1\r' "$SCRIPT" 2>&1)
	check "menu sans gum ni jq : lance le compte choisi" grep -qF "CONFIG=$(data_dir pro1)" <<<"$out"
}

test_remove() {
	ca remove pro2 </dev/null >/dev/null
	check "remove retire de la liste" test "$(ca list)" = pro1
	check "remove sans confirmation garde le dossier" test -d "$(data_dir pro2)"
	ca add pro3 >/dev/null
	with_tty o "$SCRIPT" remove pro3 >/dev/null
	check "remove confirmé supprime le dossier" test ! -e "$(data_dir pro3)"
	check "remove sur compte inexistant échoue" bash -c '! "$0" remove nope 2>/dev/null' "$SCRIPT"
	check "remove sur compte inexistant : message en anglais" bash -c \
		'"$0" remove nope 2>&1 | grep -q "no such account: nope"' "$SCRIPT"
	ca add pro5 >/dev/null
	ca add pro6 >/dev/null
	ca remove pro5 --keep-dir </dev/null >/dev/null
	check "remove --keep-dir : retiré de la liste" bash -c '! "$0" list | grep -qx pro5' "$SCRIPT"
	check "remove --keep-dir : dossier gardé sans question" test -d "$(data_dir pro5)"
	ca remove pro6 --delete-dir </dev/null >/dev/null
	check "remove --delete-dir : dossier supprimé sans terminal" test ! -e "$(data_dir pro6)"
	ca add pro7 >/dev/null
	check "remove refuse les deux options ensemble" bash -c \
		'! "$0" remove pro7 --keep-dir --delete-dir 2>/dev/null' "$SCRIPT"
	check "deux options refusées : compte et dossier intacts" bash -c \
		'"$0" list | grep -qx pro7 && test -d "$1"' "$SCRIPT" "$(data_dir pro7)"
	check "remove refuse une option inconnue" bash -c '! "$0" remove pro7 --force 2>/dev/null' "$SCRIPT"
	check "option inconnue refusée : compte et dossier intacts" bash -c \
		'"$0" list | grep -qx pro7 && test -d "$1"' "$SCRIPT" "$(data_dir pro7)"
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
test_list_empty
test_add_and_list
test_run
test_status
test_status_json_escaping
test_sessions
test_shared_mcp
test_isolation
test_errors
test_version
test_menu_keys
test_remove
rm -rf -- "$SANDBOX"
printf '\n%d tests, %d échecs\n' "$TESTS" "$FAILURES"
((FAILURES == 0))
