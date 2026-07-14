#!/usr/bin/env bash
#
# install.sh — installe / met à jour l'API Peppol Resolver (Super Popaul)
# sur un VPS Debian/Ubuntu.
#
# Idempotent : rejouable sans casse (met à jour le code, les deps, l'unit, le
# vhost ; préserve les clés existantes et la config TLS déjà générée par certbot).
#
# À lancer EN ROOT sur un VPS fraîchement livré (Debian 12+ ou Ubuntu 22.04+).
# Les images Debian d'OVH se connectent en 'debian' (sudo), pas en root ; on
# passe donc par sudo, et 'env' transmet les variables au travers de sudo.
#
#   ssh debian@VOTRE_VPS
#   curl -fsSL https://raw.githubusercontent.com/Sandjab/superpopaul/main/deploy/install.sh -o install.sh
#   sudo env DOMAIN=peppol.mondomaine.fr EMAIL=vous@exemple.fr bash install.sh
#
# DOMAIN/EMAIL non fournis ? Le script les demande interactivement (terminal
# requis). (Déjà root ? 'DOMAIN=… bash install.sh' suffit.)
#
# Paramètres (variables d'environnement OU flags) :
#   DOMAIN            (obligatoire)  nom d'hôte public, ex. peppol.mondomaine.fr
#   EMAIL            (obligatoire sauf --skip-tls)  e-mail Let's Encrypt
#   BRANCH           branche git à déployer          (défaut: main ; passez
#                    BRANCH=<branche-feature> pour déployer avant un merge)
#   REPO_URL         URL HTTPS du dépôt               (défaut: ce dépôt)
#   APP_DIR          répertoire d'install             (défaut: /opt/superpopaul)
#   SERVICE_USER     utilisateur système              (défaut: peppol)
#   KEYS_FILE        fichier de clés d'API            (défaut: /etc/peppol-api.keys)
#   RATE_LIMIT       req/min par défaut par clé       (défaut: 60)
#   MAX_CONCURRENCY  résolutions simultanées max      (défaut: 64)
#   PORT             port local du service            (défaut: 8080)
#   HARDEN_SSH       1 = couper l'auth SSH par mot de passe (défaut: 0). Opt-in,
#                    n'agit que si une clé authorized_keys existe (anti-lockout).
#
# Flags : --domain X  --email X  --branch X  --skip-tls  --harden-ssh  --help
#
set -euo pipefail

# --- valeurs par défaut ------------------------------------------------------
DOMAIN="${DOMAIN:-}"
EMAIL="${EMAIL:-}"
BRANCH="${BRANCH:-main}"
REPO_URL="${REPO_URL:-https://github.com/Sandjab/superpopaul.git}"
APP_DIR="${APP_DIR:-/opt/superpopaul}"
SERVICE_USER="${SERVICE_USER:-peppol}"
KEYS_FILE="${KEYS_FILE:-/etc/peppol-api.keys}"
RATE_LIMIT="${RATE_LIMIT:-60}"
MAX_CONCURRENCY="${MAX_CONCURRENCY:-64}"
PORT="${PORT:-8080}"
SKIP_TLS=0
HARDEN_SSH="${HARDEN_SSH:-0}"      # 1 = couper l'auth SSH par mot de passe (opt-in)

# --- flags -------------------------------------------------------------------
while [ $# -gt 0 ]; do
    case "$1" in
        --domain) DOMAIN="$2"; shift 2 ;;
        --email) EMAIL="$2"; shift 2 ;;
        --branch) BRANCH="$2"; shift 2 ;;
        --skip-tls) SKIP_TLS=1; shift ;;
        --harden-ssh) HARDEN_SSH=1; shift ;;
        --help|-h)
            sed -n '3,/^set -euo pipefail/p' "$0" | sed '$d'; exit 0 ;;
        *) echo "Option inconnue : $1" >&2; exit 2 ;;
    esac
done

# --- helpers -----------------------------------------------------------------
log()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m/!\\\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mERREUR:\033[0m %s\n' "$*" >&2; exit 1; }
# Exécute une commande en tant qu'utilisateur de service (runuser = util-linux,
# toujours présent). Évite les fichiers root-owned et l'erreur Git « dubious
# ownership » quand on relance le script (le dépôt appartient à $SERVICE_USER).
as_service() { runuser -u "$SERVICE_USER" -- "$@"; }
# Git en tant que $SERVICE_USER. GIT_TERMINAL_PROMPT=0 évite tout prompt
# interactif (échec net plutôt que blocage).
git_svc() { runuser -u "$SERVICE_USER" -- env GIT_TERMINAL_PROMPT=0 git "$@"; }

[ "$(id -u)" = 0 ] || die "À lancer en root (sudo)."

# --- saisie interactive des paramètres manquants ------------------------------
if [ -z "$DOMAIN" ] && [ -t 0 ]; then
    read -r -p "Nom d'hôte public de l'API (ex. peppol.mondomaine.fr) : " DOMAIN
fi
if [ -z "$EMAIL" ] && [ "$SKIP_TLS" = 0 ] && [ -t 0 ]; then
    read -r -p "E-mail Let's Encrypt (vide = pas de TLS, comme --skip-tls) : " EMAIL
    [ -n "$EMAIL" ] || SKIP_TLS=1
fi

[ -n "$DOMAIN" ] || die "DOMAIN manquant (ex: DOMAIN=peppol.mondomaine.fr)."
if [ "$SKIP_TLS" = 0 ] && [ -z "$EMAIL" ]; then
    die "EMAIL manquant (pour Let's Encrypt), ou passez --skip-tls."
fi

export DEBIAN_FRONTEND=noninteractive

# --- 1. paquets système ------------------------------------------------------
log "Mise à jour du système et installation des paquets…"
apt-get update -q
apt-get -y -q full-upgrade
apt-get install -y -q git python3-venv nginx certbot python3-certbot-nginx \
                      ufw fail2ban unattended-upgrades curl ca-certificates

# --- 2. mises à jour de sécurité automatiques --------------------------------
log "Activation des mises à jour de sécurité automatiques…"
cat > /etc/apt/apt.conf.d/20auto-upgrades <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
EOF

# --- 3. pare-feu + fail2ban --------------------------------------------------
log "Configuration du pare-feu (ufw) et de fail2ban…"
ufw default deny incoming  >/dev/null    # explicite (= défaut ufw, remis au propre)
ufw default allow outgoing >/dev/null
ufw allow OpenSSH        >/dev/null 2>&1 || ufw allow 22/tcp >/dev/null
ufw allow 'Nginx Full'   >/dev/null 2>&1 || { ufw allow 80/tcp >/dev/null; ufw allow 443/tcp >/dev/null; }
ufw --force enable       >/dev/null
systemctl enable --now fail2ban >/dev/null 2>&1 || true

# --- 3.b durcissement SSH (opt-in : HARDEN_SSH=1 / --harden-ssh) --------------
# Coupe l'auth par mot de passe (drop-in, sans toucher au sshd_config d'origine).
# Garde-fou anti-lockout : ne s'exécute QUE si une clé publique est déjà
# autorisée quelque part — sinon on ne risque pas de vous verrouiller dehors.
if [ "$HARDEN_SSH" = 1 ]; then
    have_key=0
    for f in /root/.ssh/authorized_keys /home/*/.ssh/authorized_keys; do
        if [ -s "$f" ]; then have_key=1; break; fi
    done
    if [ "$have_key" = 0 ]; then
        warn "HARDEN_SSH : aucune clé authorized_keys trouvée — durcissement SSH IGNORÉ (anti-lockout)."
    else
        log "Durcissement SSH : désactivation de l'authentification par mot de passe…"
        install -d -m 755 /etc/ssh/sshd_config.d
        cat > /etc/ssh/sshd_config.d/99-peppol-hardening.conf <<'EOF'
# Déposé par superpopaul/deploy/install.sh (HARDEN_SSH=1).
PasswordAuthentication no
KbdInteractiveAuthentication no
EOF
        if sshd -t 2>/dev/null; then
            systemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null || true
            log "SSH durci (PasswordAuthentication no). Vérifiez une nouvelle session par clé avant de fermer celle-ci."
        else
            warn "sshd -t a échoué — durcissement SSH annulé, drop-in retiré."
            rm -f /etc/ssh/sshd_config.d/99-peppol-hardening.conf
        fi
    fi
fi

# --- 4. utilisateur de service ----------------------------------------------
if ! id -u "$SERVICE_USER" >/dev/null 2>&1; then
    log "Création de l'utilisateur système '$SERVICE_USER'…"
    useradd --system --home "$APP_DIR" --shell /usr/sbin/nologin "$SERVICE_USER"
else
    log "Utilisateur '$SERVICE_USER' déjà présent."
fi

# --- 5. code applicatif (clone partiel + sparse) -----------------------------
# On ne récupère que les fichiers utiles au runtime, pas tout le dépôt (qui
# contient le client Tauri, les clients CLI, la doc… inutiles ici) :
#   - clone PARTIEL (--filter=blob:none) : les blobs ne sont téléchargés qu'à la
#     demande, donc seuls ceux des fichiers ci-dessous descendent ;
#   - sparse-checkout : seuls ces chemins peuplent le working tree.
# `git pull`/reset restent fonctionnels pour les mises à jour (re-run du script).
# Le dépôt appartient à $SERVICE_USER et git tourne sous cet utilisateur : pas de
# fichiers root-owned, et pas d'erreur Git « dubious ownership » au re-run.
# Chemins ancrés (slash de tête) = motifs sparse « no-cone » précis, sans warning.
APP_FILES="/server/ /deploy/"   # requis au runtime
mkdir -p "$APP_DIR"
chown "$SERVICE_USER:$SERVICE_USER" "$APP_DIR"
if [ -d "$APP_DIR/.git" ]; then
    log "Mise à jour du dépôt ($BRANCH, sparse)…"
    # shellcheck disable=SC2086  # word-splitting voulu sur la liste de chemins
    git_svc -C "$APP_DIR" sparse-checkout set --no-cone $APP_FILES
    git_svc -C "$APP_DIR" fetch --quiet --filter=blob:none origin "$BRANCH"
    git_svc -C "$APP_DIR" checkout --quiet "$BRANCH"
    git_svc -C "$APP_DIR" reset --hard --quiet "origin/$BRANCH"
else
    log "Clonage partiel (sparse) dans $APP_DIR ($BRANCH)…"
    git_svc clone --quiet --no-checkout --filter=blob:none \
        --branch "$BRANCH" "$REPO_URL" "$APP_DIR"
    # shellcheck disable=SC2086  # word-splitting voulu sur la liste de chemins
    git_svc -C "$APP_DIR" sparse-checkout set --no-cone $APP_FILES
    git_svc -C "$APP_DIR" checkout --quiet "$BRANCH"
fi

# --- 6. environnement Python + dépendances -----------------------------------
if [ ! -x "$APP_DIR/.venv/bin/python" ]; then
    log "Création de l'environnement virtuel Python…"
    as_service python3 -m venv "$APP_DIR/.venv"
fi
log "Installation/màj des dépendances (server/requirements.txt)…"
as_service "$APP_DIR/.venv/bin/pip" install --quiet --upgrade pip
as_service "$APP_DIR/.venv/bin/pip" install --quiet --upgrade \
    -r "$APP_DIR/server/requirements.txt"

chown -R "$SERVICE_USER:$SERVICE_USER" "$APP_DIR"   # ceinture + bretelles

# --- 7. fichier de clés d'API ------------------------------------------------
GENERATED_KEY=""
if [ ! -f "$KEYS_FILE" ]; then
    log "Génération d'une première clé d'API…"
    GENERATED_KEY="$("$APP_DIR/.venv/bin/python" "$APP_DIR/server/peppol_api.py" --gen-key)"
    install -m 600 -o "$SERVICE_USER" -g "$SERVICE_USER" /dev/null "$KEYS_FILE"
    printf '# label=CLE [rate req/min] [burst] — voir deploy/peppol-api.keys.example\nadmin=%s\n' \
        "$GENERATED_KEY" > "$KEYS_FILE"
    chown "$SERVICE_USER:$SERVICE_USER" "$KEYS_FILE"
    chmod 600 "$KEYS_FILE"
else
    log "Fichier de clés déjà présent ($KEYS_FILE) — inchangé."
fi

# --- 8. service systemd ------------------------------------------------------
# Source unique : deploy/peppol-api.service du dépôt (présent via le
# sparse-checkout de /deploy/). On substitue les valeurs paramétrables ; tout le
# reste (env DNS, durcissement…) passe tel quel — pas de copie divergente ici.
UNIT_SRC="$APP_DIR/deploy/peppol-api.service"
[ -f "$UNIT_SRC" ] || die "Unit introuvable : $UNIT_SRC (sparse-checkout incomplet ?)"
log "Installation de l'unit systemd (depuis deploy/peppol-api.service)…"
sed -e "s|^User=.*|User=$SERVICE_USER|" \
    -e "s|^Group=.*|Group=$SERVICE_USER|" \
    -e "s|^WorkingDirectory=.*|WorkingDirectory=$APP_DIR|" \
    -e "s|^ExecStart=.*|ExecStart=$APP_DIR/.venv/bin/python $APP_DIR/server/peppol_api.py|" \
    -e "s|^Environment=PEPPOL_API_PORT=.*|Environment=PEPPOL_API_PORT=$PORT|" \
    -e "s|^Environment=PEPPOL_API_KEYS_FILE=.*|Environment=PEPPOL_API_KEYS_FILE=$KEYS_FILE|" \
    -e "s|^Environment=PEPPOL_API_RATE_LIMIT=.*|Environment=PEPPOL_API_RATE_LIMIT=$RATE_LIMIT|" \
    -e "s|^Environment=PEPPOL_API_MAX_CONCURRENCY=.*|Environment=PEPPOL_API_MAX_CONCURRENCY=$MAX_CONCURRENCY|" \
    "$UNIT_SRC" > /etc/systemd/system/peppol-api.service

systemctl daemon-reload
systemctl enable --now peppol-api >/dev/null 2>&1 || true
systemctl restart peppol-api
sleep 1
if ! curl -fs --noproxy '*' "http://127.0.0.1:$PORT/health" >/dev/null; then
    warn "Le service ne répond pas encore sur /health — vérifiez : journalctl -u peppol-api -e"
else
    log "Service en ligne (http://127.0.0.1:$PORT/health)."
fi

# --- 9. vhost nginx (amorçage HTTP) ------------------------------------------
# On ne réécrit le vhost QUE s'il est absent OU si son server_name ne correspond
# plus à $DOMAIN (changement de domaine) : sinon on préserve la config TLS déjà
# générée par certbot. Après un changement de domaine, certbot (§10) reprend la
# main pour émettre le certificat du nouveau nom.
NGINX_VHOST=/etc/nginx/sites-available/peppol-api
if [ ! -f "$NGINX_VHOST" ] || ! grep -qF "server_name $DOMAIN;" "$NGINX_VHOST"; then
    log "Écriture du vhost nginx (HTTP) pour $DOMAIN…"
    cat > "$NGINX_VHOST" <<EOF
server {
    listen 80;
    listen [::]:80;
    server_name $DOMAIN;

    location = /health { proxy_pass http://127.0.0.1:$PORT; }
    location / {
        proxy_pass http://127.0.0.1:$PORT;
        proxy_set_header Host              \$host;
        proxy_set_header X-Real-IP         \$remote_addr;
        proxy_set_header X-Forwarded-For   \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_read_timeout 60s;
    }
}
EOF
    ln -sf "$NGINX_VHOST" /etc/nginx/sites-enabled/peppol-api
    rm -f /etc/nginx/sites-enabled/default
else
    log "vhost nginx déjà à jour pour $DOMAIN — inchangé (config TLS certbot préservée)."
fi
nginx -t && systemctl reload nginx

# --- 10. certificat TLS ------------------------------------------------------
if [ "$SKIP_TLS" = 1 ]; then
    warn "TLS ignoré (--skip-tls). L'API répond en HTTP sur le port 80."
elif [ -d "/etc/letsencrypt/live/$DOMAIN" ]; then
    # Le certificat existe déjà : on le (ré)installe dans le vhost courant. C'est
    # indispensable après une régénération du vhost (changement de domaine) où
    # le TLS n'est plus câblé ; idempotent si déjà en place.
    log "Certificat déjà présent pour $DOMAIN — (ré)installation dans nginx…"
    if certbot install --cert-name "$DOMAIN" --nginx --redirect --non-interactive; then
        log "TLS en place (HTTPS + redirection 80→443)."
    else
        warn "certbot install a échoué — vérifiez que le vhost a bien server_name $DOMAIN."
    fi
else
    log "Obtention du certificat Let's Encrypt pour $DOMAIN…"
    if certbot --nginx -d "$DOMAIN" --agree-tos -m "$EMAIL" --redirect \
               --non-interactive --no-eff-email; then
        log "TLS activé (HTTPS + redirection 80→443)."
    else
        warn "certbot a échoué. Causes fréquentes : DNS pas encore propagé"
        warn "(dig +short A $DOMAIN doit renvoyer l'IP du VPS) ou port 80 fermé."
        warn "L'API reste accessible en HTTP ; relancez ce script après propagation."
    fi
fi

# --- récapitulatif -----------------------------------------------------------
echo
log "Installation terminée."
if [ -n "$GENERATED_KEY" ]; then
    echo "  Clé d'API générée (label 'admin') : $GENERATED_KEY"
    echo "  (stockée dans $KEYS_FILE — ajoutez/éditez vos clients puis :"
    echo "   systemctl restart peppol-api)"
fi
SCHEME="https"; [ "$SKIP_TLS" = 1 ] && SCHEME="http"
echo "  Test :"
echo "    curl $SCHEME://$DOMAIN/health"
echo "    curl -H \"X-API-Key: <CLE>\" $SCHEME://$DOMAIN/resolve/0225:000122308"
echo "    doc interactive : $SCHEME://$DOMAIN/docs"
