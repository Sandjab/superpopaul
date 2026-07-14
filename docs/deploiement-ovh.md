# Déploiement de l'API Peppol Resolver sur un VPS OVHcloud

Mode opératoire complet pour exposer `server/peppol_api.py` sur
`https://peppol.mondomaine.fr`, derrière nginx (TLS Let's Encrypt) et piloté
par systemd.

> **`peppol.mondomaine.fr` et `vous@exemple.fr` sont des valeurs d'exemple** :
> remplacez-les partout par votre nom d'hôte public et votre e-mail (ou
> exportez-les une fois pour toutes — les commandes ci-dessous les reprennent) :
>
> ```bash
> export DOMAIN=peppol.mondomaine.fr EMAIL=vous@exemple.fr
> ```
>
> Le script `deploy/install.sh` les lit dans l'environnement, et les demande
> interactivement s'ils manquent.

> Gamme et prix relevés en juillet 2026 sur la page officielle
> (<https://www.ovhcloud.com/fr/vps/>). Le catalogue OVH évolue — **vérifiez les
> specs et tarifs exacts dans le configurateur au moment de commander**.

---

## 0. Choisir la config VPS

### Ce que consomme réellement l'API

Le service est **I/O-bound** : chaque résolution passe l'essentiel de son temps à
attendre le réseau (DNS NAPTR + requêtes HTTP vers les SMP Peppol). Empreinte
CPU/RAM quasi nulle (serveur `http.server` stdlib + un peu de parsing XML/cert).
Le facteur limitant est la latence amont et le rate-limit Peppol, **pas** la
taille du VPS. Conclusion : **le plus petit VPS du catalogue suffit largement.**

### Gamme OVH VPS 2027

La gamme 2027 s'appelle **VPS-1 à VPS-4** (stockage SSD NVMe, 1 IPv4 dédiée +
IPv6 incluses, trafic illimité, anti-DDoS inclus, **sauvegardes quotidiennes
incluses** avec rétention glissante de 7 jours, console KVM intégrée).

| Modèle | vCores | RAM | NVMe | Bande passante | Prix/mois HT (TTC) |
|--------|-------:|----:|-----:|---------------:|-------------------:|
| **VPS-1** | 2 | 4 Go | 40 Go | 500 Mbit/s | 3,81 € (4,57 €) |
| **VPS-2** | 4 | 8 Go | 75 Go | 1 Gbit/s | 7,21 € (8,65 €) |
| **VPS-3** | 6 | 12 Go | 100 Go | 2 Gbit/s | 10,40 € (12,48 €) |
| **VPS-4** | 8 | 24 Go | 200 Go | 3 Gbit/s | 19,96 € (23,95 €) |

Les configurations sont upgradables sans interruption de service ; options
payantes : sauvegarde avancée, Load Balancer, IP additionnelles.

### Recommandation

| Besoin | Choix | Pourquoi |
|--------|-------|----------|
| **API seule, usage perso/partenaires** | **VPS-1** | 2 vCores / 4 Go suffisent largement pour cette charge I/O-bound. Le bon rapport prix/marge. |
| Marge confort (logs volumineux, batchs de résolution en masse, 2-3 autres petits services sur la même machine) | VPS-2 | Double CPU, RAM et disque pour ~3,5 €/mois de plus. |
| Inutile ici | VPS-3+ | Aucun intérêt pour cette API : vous paieriez du CPU/RAM jamais utilisés. |

**En pratique : VPS-1** (ou VPS-2 si vous voulez de la place pour grandir).

**Autres choix à la commande :**
- **OS : Debian 12 « bookworm »** (Python 3.11, ce qui suit est écrit pour lui) ;
  Debian 13 ou Ubuntu 24.04 LTS conviennent aussi.
- **Datacenter : Gravelines ou Strasbourg (France)** — faible latence vers
  l'infra Peppol européenne et données qui restent en France (contexte CTC-FR).
- **Clé SSH** : enregistrez votre clé publique dans l'espace client puis
  sélectionnez-la à l'installation (procédure détaillée en **§1**), pour vous
  connecter sans mot de passe.
- **Sauvegardes** : les sauvegardes quotidiennes (rétention 7 jours) sont
  désormais incluses — rien à cocher ; l'option « sauvegarde avancée » n'est
  utile que pour des besoins de rétention plus longs.

---

## 1. Commander le VPS

### 1.a Préparer la clé SSH (recommandé, avant la commande)

Chez OVH la clé SSH se **sélectionne à l'étape d'installation de l'OS** (à la
commande ou lors d'une réinstallation), pas dans un champ libre au paiement. Deux
options : soit **coller la clé directement** à cette étape, soit la **pré-enregistrer
dans l'espace client** pour la retrouver dans un menu déroulant (point 2 ci-dessous).

1. **Générer une paire de clés** (si vous n'en avez pas), sur votre poste :
   ```bash
   ssh-keygen -t ed25519 -C "mon-vps"
   cat ~/.ssh/id_ed25519.pub      # la clé PUBLIQUE à copier (ssh-ed25519 …)
   ```
   La clé **privée** (`~/.ssh/id_ed25519`) ne quitte jamais votre machine.
2. **Enregistrer la clé publique chez OVH** (facultatif — c'est un confort ;
   vous pourrez sinon coller la clé directement à l'installation, cf. §1.b) :
   espace client → (en haut à droite) votre identifiant → **« Mes offres &
   services »** → page **« Mes services »**, onglet **« Clés SSH »** →
   **Ajouter une clé SSH** → type **« Dédié »** (⚠️ libellé exact *Dédié* ; ce
   groupe couvre aussi les **VPS**). Donnez un **label** (`mon-vps`) et collez
   la chaîne de clé **publique**. Formats acceptés : **Ed25519** et **RSA**.

### 1.b Commander

1. <https://www.ovhcloud.com/fr/vps/> → configurez **VPS-1**, OS **Debian 12**,
   datacenter **France**, validez.
2. À l'étape **Système d'exploitation**, si le menu **« Clé SSH à préinstaller »**
   apparaît : choisissez `mon-vps` et cochez **« Je ne souhaite pas recevoir
   mes codes d'authentification VPS par e-mail »**. (Vous pouvez aussi coller la
   clé dans le champ **« Votre clé SSH publique »**.)
3. À la livraison (quelques minutes), OVH fournit l'**IPv4** et l'**IPv6** du VPS
   (e-mail + espace client → *Bare Metal Cloud / VPS*).

Notez ces deux adresses, appelées ci-après `IPV4_DU_VPS` et `IPV6_DU_VPS`.

> **Si l'étape « Clé SSH » n'apparaît pas** (variable selon l'OS/le tunnel) —
> deux options :
> - **Réinstaller le VPS** depuis l'espace client (VPS → *⋯ / Réinstaller*) :
>   l'assistant de réinstallation propose bien le menu de clé SSH.
> - **Ou** se connecter une première fois avec le **mot de passe reçu par
>   e-mail**, puis pousser la clé (sur les images Debian d'OVH, l'utilisateur
>   par défaut est **`debian`** — pas `root`) :
>   `ssh-copy-id -i ~/.ssh/id_ed25519.pub debian@IPV4_DU_VPS`
>   (l'**Annexe A** désactive ensuite l'authentification par mot de passe).

---

## 2. Pointer le domaine vers le VPS (DNS)

Exemple avec un domaine dont la zone DNS est gérée chez OVH (adaptez chez votre
registrar sinon) :

1. Espace client OVH → **Web Cloud** → **Noms de domaine** → votre domaine →
   onglet **Zone DNS**.
2. **Ajouter une entrée** de type **A** :
   - Sous-domaine : `peppol` (ou celui choisi pour l'API)
   - Cible : `IPV4_DU_VPS`
   - TTL : par défaut
3. **Ajouter une entrée** de type **AAAA** (recommandé, IPv6) :
   - Sous-domaine : `peppol`
   - Cible : `IPV6_DU_VPS`
4. Enregistrez. La propagation prend de quelques minutes à ~1 h.

Vérifiez depuis votre poste :

```bash
dig +short A    "$DOMAIN"     # doit renvoyer IPV4_DU_VPS
dig +short AAAA "$DOMAIN"     # doit renvoyer IPV6_DU_VPS
```

> N'allez pas plus loin (certificat TLS) tant que `dig` ne renvoie pas la bonne IP.

---

## 🚀 Chemin rapide : le script `deploy/install.sh`

Une fois les étapes **1** (VPS commandé) et **2** (DNS propagé) faites, tout le
reste (durcissement, déploiement, systemd, nginx, TLS) est automatisé et
**idempotent** (rejouable sans casse). Connecté en **`debian`** sur le VPS
(images Debian d'OVH), on récupère **le seul `install.sh`** (pas besoin de cloner
le dépôt à la main — le script fait ensuite un clone *partiel*) et on le lance en
root via `sudo` (`env` transmet les variables au travers de `sudo`) :

```bash
ssh debian@peppol.mondomaine.fr

curl -fsSL https://raw.githubusercontent.com/Sandjab/superpopaul/main/deploy/install.sh -o install.sh
sudo env DOMAIN=peppol.mondomaine.fr EMAIL=vous@exemple.fr bash install.sh
```

(Sans `DOMAIN`/`EMAIL` dans l'environnement, le script les demande
interactivement.)

Le script installe les paquets, configure `ufw`/`fail2ban`/MAJ auto, crée
l'utilisateur `peppol`, **récupère uniquement `server/` et `deploy/`** (clone
`--filter=blob:none` + sparse-checkout, pas le client Tauri ni la doc), crée le
venv + deps, **génère une première clé d'API** (affichée à la fin), installe le
service systemd et le vhost nginx, puis obtient le certificat Let's Encrypt.
Options utiles : `RATE_LIMIT=120`, `--skip-tls` (si le DNS n'est pas encore
propagé — relancez le script plus tard pour le TLS), `BRANCH=<branche>` pour
déployer une branche autre que `main`, `--harden-ssh` (ou `HARDEN_SSH=1`) pour
couper aussi l'auth SSH par mot de passe (cf. Annexe A — n'agit que si une clé
est déjà en place, garde-fou anti-lockout).

Les sections **3 à 10** ci-dessous détaillent ces mêmes étapes **à la main** — à
lire pour comprendre, adapter, ou dépanner. Si vous utilisez le script, sautez à
la section **9. Vérification**.

---

## 3. Première connexion et durcissement

Connexion en **`debian`** (utilisateur par défaut des images Debian d'OVH ;
avec la clé SSH préparée en §1, sinon le mot de passe reçu par e-mail) :

```bash
ssh debian@"$DOMAIN"      # ou ssh debian@IPV4_DU_VPS
```

Les commandes d'administration ci-dessous (sections 3 à 8) supposent un **shell
root** — le plus simple est d'en ouvrir un une fois :

```bash
sudo -i        # devient root ; sinon, préfixez chaque commande par 'sudo'
```

Mises à jour + outils de base :

```bash
apt update && apt full-upgrade -y
apt install -y git python3-venv nginx fail2ban ufw certbot python3-certbot-nginx \
               unattended-upgrades
```

Mises à jour de sécurité automatiques :

```bash
dpkg-reconfigure -plow unattended-upgrades   # répondre « Oui »
```

Pare-feu (autorise SSH + HTTP + HTTPS, refuse le reste) :

```bash
ufw default deny incoming
ufw default allow outgoing
ufw allow OpenSSH
ufw allow 'Nginx Full'          # ouvre 80 + 443
ufw enable
ufw status verbose
```

fail2ban (protège SSH contre la force brute) est actif par défaut après
installation :

```bash
systemctl enable --now fail2ban
```

Durcissement SSH (désactiver l'authentification par mot de passe une fois votre
clé en place) — voir l'**annexe A**. Sur les images Debian d'OVH, l'utilisateur
`debian` (sudo) joue déjà le rôle d'admin non-root et le login root SSH est
désactivé par défaut.

---

## 4. Déployer l'application

On installe le code dans `/opt/superpopaul`, exécuté par un utilisateur système
dédié `peppol` (aucun shell, aucun privilège). Au runtime, l'API n'a besoin que
de **`server/`** et **`deploy/`** — on fait donc un **clone partiel + sparse**
plutôt que de rapatrier tout le dépôt (client Tauri, clients CLI, doc…).
`git pull` reste possible pour les mises à jour.

```bash
# Helper : git en tant que 'peppol', sans prompt interactif (runuser = util-linux).
gitp() { runuser -u peppol -- env GIT_TERMINAL_PROMPT=0 git "$@"; }

# Utilisateur de service (git/pip tournent sous lui : pas de fichiers root-owned,
# pas d'erreur Git « dubious ownership »).
useradd --system --home /opt/superpopaul --shell /usr/sbin/nologin peppol
install -d -o peppol -g peppol /opt/superpopaul

# Code : clone partiel (blobs à la demande) + sparse-checkout des seuls chemins utiles
gitp clone --no-checkout --filter=blob:none \
     https://github.com/Sandjab/superpopaul.git /opt/superpopaul
gitp -C /opt/superpopaul sparse-checkout set --no-cone /server/ /deploy/
gitp -C /opt/superpopaul checkout main   # ou une autre branche

# Environnement Python isolé + dépendances (dnspython, cryptography)
runuser -u peppol -- python3 -m venv /opt/superpopaul/.venv
runuser -u peppol -- /opt/superpopaul/.venv/bin/pip install --upgrade pip
runuser -u peppol -- /opt/superpopaul/.venv/bin/pip install \
     -r /opt/superpopaul/server/requirements.txt
```

Test rapide (résolveur en direct, sans le serveur) :

```bash
runuser -u peppol -- /opt/superpopaul/.venv/bin/python \
     /opt/superpopaul/server/peppol_resolver.py 0225:000122308 --ap-only
```

Vous devez voir la résolution SML→SMP aboutir (`SML lookup : OK`, une `SMP URL`).

---

## 5. Créer les clés d'API

Générez une clé par client et déposez-les dans `/etc/peppol-api.keys` (format
`label=CLE [rate] [burst]`, voir `deploy/peppol-api.keys.example`) :

```bash
# Générer des clés
/opt/superpopaul/.venv/bin/python /opt/superpopaul/server/peppol_api.py --gen-key   # x autant que de clients

# Créer le fichier (600, lisible par le seul user peppol)
install -m 600 -o peppol -g peppol /dev/null /etc/peppol-api.keys
$EDITOR /etc/peppol-api.keys
```

Exemple de contenu :

```
# label=CLE                         rate(req/min)  burst
moi=Xy9aBc...                                             # défaut global
partenaire=Zk3dEf...                600            100
monitoring=Qw7gHi...                0                     # illimité (sonde interne)
```

Le rate-limit par défaut (clés sans valeur propre) est fixé par
`PEPPOL_API_RATE_LIMIT` dans l'unit systemd (voir plus bas).

---

## 6. Service systemd

L'unit fournie écoute **uniquement en local** (`127.0.0.1:8080`) : nginx fait le
front public en TLS.

```bash
cp /opt/superpopaul/deploy/peppol-api.service /etc/systemd/system/
```

Vérifiez/ajustez les variables dans `/etc/systemd/system/peppol-api.service`
(section `[Service]`) — les valeurs par défaut conviennent :

```ini
Environment=PEPPOL_API_HOST=127.0.0.1
Environment=PEPPOL_API_PORT=8080
Environment=PEPPOL_API_KEYS_FILE=/etc/peppol-api.keys
Environment=PEPPOL_API_RATE_LIMIT=60          # défaut req/min par clé
Environment=PEPPOL_API_MAX_CONCURRENCY=64     # fetchs SMP simultanés
Environment=PEPPOL_API_DNS_SERVER=8.8.8.8     # résolveur public (celui du VPS défaille sous rafale NAPTR)
Environment=PEPPOL_API_DNS_FALLBACK=1.1.1.1   # secours si le principal échoue après retries
```

> `install.sh` installe cette même unit (celle du dépôt, `deploy/peppol-api.service`)
> en y substituant utilisateur, chemins, port, rate-limit et concurrence — il n'y a
> qu'une seule source de vérité pour l'unit.

Démarrage + activation au boot :

```bash
systemctl daemon-reload
systemctl enable --now peppol-api
systemctl status peppol-api --no-pager
```

Test local (sur le VPS) :

```bash
curl -s http://127.0.0.1:8080/health          # {"status":"ok"}
curl -s -H "X-API-Key: <UNE_CLE>" \
     http://127.0.0.1:8080/resolve/0225:000122308
```

Logs :

```bash
journalctl -u peppol-api -f
```

---

## 7. Reverse-proxy nginx (vhost HTTP d'amorçage)

On pose d'abord un vhost **HTTP simple** : il permet à `nginx -t` de passer (pas
encore de certificat) et sert le challenge ACME de l'étape 8. certbot le
transformera ensuite en HTTPS.

```bash
rm -f /etc/nginx/sites-enabled/default        # retire le vhost par défaut

cat > /etc/nginx/sites-available/peppol-api <<EOF
server {
    listen 80;
    listen [::]:80;
    server_name $DOMAIN;

    location = /health {
        proxy_pass http://127.0.0.1:8080;
    }
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host              \$host;
        proxy_set_header X-Real-IP         \$remote_addr;
        proxy_set_header X-Forwarded-For   \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_read_timeout 60s;
    }
}
EOF

ln -s /etc/nginx/sites-available/peppol-api /etc/nginx/sites-enabled/
nginx -t && systemctl reload nginx
```

À ce stade `http://peppol.mondomaine.fr/health` répond déjà (en clair, port 80).

---

## 8. Certificat TLS Let's Encrypt

DNS propagé + port 80 ouvert, certbot obtient le certificat et **réécrit
automatiquement le vhost** pour le HTTPS + la redirection 80→443 :

```bash
certbot --nginx -d "$DOMAIN" \
        --agree-tos -m "$EMAIL" --redirect --no-eff-email
```

Le renouvellement est automatique (timer systemd `certbot`) ; test à blanc :

```bash
certbot renew --dry-run
```

> **Optionnel — durcissement du front.** Le vhost généré par certbot est
> fonctionnel mais minimal. Pour ajouter les en-têtes de sécurité et le
> rate-limit de bordure de l'exemple du repo
> (`deploy/nginx-peppol-api.conf`), reportez dans le vhost généré la directive
> `limit_req_zone …` (au niveau `http`, p.ex. dans `/etc/nginx/conf.d/`) et,
> dans le `server{}` 443, les lignes `add_header …` et `limit_req …`, puis
> `nginx -t && systemctl reload nginx`.

---

## 9. Vérification de bout en bout

Depuis votre poste :

```bash
# Public, sans clé
curl -s https://"$DOMAIN"/health                                  # {"status":"ok"}

# Sans clé sur /resolve -> 401
curl -s -o /dev/null -w "%{http_code}\n" \
     https://"$DOMAIN"/resolve/0225:000122308                     # 401

# Avec clé -> réponse simple
curl -s -H "X-API-Key: <UNE_CLE>" \
     https://"$DOMAIN"/resolve/0225:000122308 | python3 -m json.tool

# Batch : jusqu'à 500 adressages en une requête
curl -s -X POST -H "X-API-Key: <UNE_CLE>" -H "Content-Type: application/json" \
     -d '{"participants":["0225:000122308","0225:931153688"]}' \
     https://"$DOMAIN"/resolve/batch | python3 -m json.tool

# Doc interactive dans le navigateur
#   https://peppol.mondomaine.fr/docs
```

Réponse attendue :

```json
{
  "participant_id": "iso6523-actorid-upis::0225:000122308",
  "exists": true,
  "pa": { "code": "PFR000123", "name": "…", "country": "FR" },
  "supports_extended_ctc_fr": true
}
```

---

## 10. Exploitation au quotidien

> Ces commandes d'administration s'exécutent en root : connecté en `debian`,
> préfixez-les par `sudo` (ou ouvrez un shell root avec `sudo -i`).

**Mettre à jour l'application :**

```bash
# Le plus simple : relancer install.sh (idempotent) — met à jour code + deps + config :
sudo env DOMAIN="$DOMAIN" EMAIL="$EMAIL" bash /opt/superpopaul/deploy/install.sh

# Sinon, le pull à la main :
sudo runuser -u peppol -- env GIT_TERMINAL_PROMPT=0 git -C /opt/superpopaul pull
sudo runuser -u peppol -- /opt/superpopaul/.venv/bin/pip install --upgrade \
     -r /opt/superpopaul/server/requirements.txt
sudo systemctl restart peppol-api
```

**Ajouter / révoquer une clé :** éditez `/etc/peppol-api.keys` puis
`systemctl restart peppol-api` (le fichier n'est relu qu'au démarrage).

**Changer le rate-limit par défaut :** modifiez `PEPPOL_API_RATE_LIMIT` dans
l'unit, `systemctl daemon-reload && systemctl restart peppol-api`.

**Changer de domaine** (ex. `api.mondomaine.fr` → `peppol.mondomaine.fr`) :
1. **DNS** : ajoutez le nouveau nom dans la zone (A → IPv4, AAAA → IPv6) et
   attendez la propagation (`dig +short A <nouveau-nom>`).
2. **Relancez `install.sh` avec le nouveau `DOMAIN`** — il détecte que le
   `server_name` du vhost a changé, régénère le vhost et fait émettre un nouveau
   certificat Let's Encrypt pour ce nom :
   ```bash
   sudo env DOMAIN=<nouveau-nom> EMAIL="$EMAIL" bash /opt/superpopaul/deploy/install.sh
   ```
   L'ancien nom continue de résoudre tant que son enregistrement DNS existe ;
   supprimez-le dans la zone si vous voulez couper l'ancienne URL. L'ancien
   certificat inutilisé peut être retiré avec `sudo certbot delete --cert-name
   <ancien-nom>`.

**Migrer vers un nouveau VPS** (changement de gamme ou de datacenter — l'upgrade
en place n'est pas toujours possible, le stock d'une zone pouvant être épuisé) :

L'API est **stateless** (rate-limiter et sémaphores en mémoire) : le seul état à
emporter est `/etc/peppol-api.keys`. Tout le reste se reconstruit via `install.sh`.

1. **J-1 : baissez le TTL** des enregistrements A/AAAA du sous-domaine dans la
   zone DNS (p. ex. 60 s) pour raccourcir la fenêtre de bascule.
2. **Commandez le nouveau VPS** (Debian 12, clé SSH — cf. §0-1) et installez-y
   l'API **avec `--skip-tls`** (le DNS pointe encore vers l'ancien VPS, le
   challenge certbot échouerait) :
   ```bash
   sudo env DOMAIN="$DOMAIN" bash install.sh --skip-tls --harden-ssh
   ```
3. **Copiez les clés d'API** depuis l'ancien VPS (sinon toutes les clés clients
   cassent), puis redémarrez le service :
   ```bash
   ssh debian@ANCIENNE_IP "sudo cat /etc/peppol-api.keys" \
     | ssh debian@NOUVELLE_IP "sudo tee /etc/peppol-api.keys > /dev/null"
   ssh debian@NOUVELLE_IP "sudo chown peppol:peppol /etc/peppol-api.keys \
     && sudo chmod 600 /etc/peppol-api.keys && sudo systemctl restart peppol-api"
   ```
4. **Validez à froid** sur le nouveau VPS : `curl -s http://127.0.0.1:8080/health`,
   un `/resolve` avec une vraie clé, et `systemctl cat peppol-api` (les lignes
   `PEPPOL_API_DNS_SERVER=8.8.8.8` et `PEPPOL_API_DNS_FALLBACK=1.1.1.1` doivent
   y figurer).
5. **Basculez le DNS** (A → nouvelle IPv4, AAAA → nouvelle IPv6) et attendez que
   `dig +short A "$DOMAIN"` renvoie la nouvelle IP.
6. **Relancez `install.sh` sans `--skip-tls`** (avec `EMAIL=…`) : certbot émet le
   certificat et câble HTTPS + redirection 80→443. Coupure HTTPS ≈ TTL + ~1 min
   d'émission.
7. **Vérifiez de bout en bout** (§9), pointez le healthcheck externe sur la
   nouvelle machine si besoin, puis gardez l'ancien VPS quelques jours (traînards
   DNS), remontez le TTL et résiliez.

**Logs & supervision :**
- `journalctl -u peppol-api -f` (application), `journalctl -u nginx` (front).
- Healthcheck externe : pointez un moniteur (UptimeRobot, etc.) sur
  `https://<votre-domaine>/health` (non authentifié, non rate-limité).

**Sauvegardes :** les sauvegardes quotidiennes incluses (rétention 7 jours)
suffisent ; sauvegardez surtout `/etc/peppol-api.keys` (hors du VPS, chiffré).

---

## Annexe A — Durcissement SSH (recommandé)

Sur les images Debian d'OVH, l'utilisateur **`debian`** (sudo) est déjà votre
admin non-root, et le login **root SSH est désactivé par défaut**. Une fois votre
clé publique en place sur ce compte (§1), il reste surtout à **couper
l'authentification par mot de passe**.

Dans `/etc/ssh/sshd_config` (en root), assurez-vous d'avoir :

```
PermitRootLogin no
PasswordAuthentication no
```

puis rechargez SSH :

```bash
sudo systemctl restart ssh
```

⚠️ **Gardez votre session en cours ouverte** et vérifiez dans un **nouveau**
terminal que `ssh debian@"$DOMAIN"` fonctionne (par clé) avant de la
fermer — sinon vous risquez de vous verrouiller dehors.

> **Automatisable** : `install.sh` peut poser ce durcissement pour vous avec
> `HARDEN_SSH=1` (ou `--harden-ssh`). Il écrit un drop-in
> `/etc/ssh/sshd_config.d/99-peppol-hardening.conf` (`PasswordAuthentication no`)
> **uniquement si** une clé `authorized_keys` est déjà présente (garde-fou
> anti-lockout), valide la config (`sshd -t`) puis recharge SSH.

## Annexe B — Dépannage

| Symptôme | Piste |
|----------|-------|
| `certbot` échoue (challenge HTTP) | DNS pas encore propagé (`dig +short A <domaine>`), ou port 80 fermé (`ufw status`). |
| `502 Bad Gateway` | Le service est down : `systemctl status peppol-api`, `journalctl -u peppol-api`. |
| `/resolve` renvoie `exists:true` mais `supports_extended_ctc_fr:null` + `note` | Le SMP a refusé le catalogue (403) ou lookup incomplet ; normal pour certains participants. |
| Résolutions en échec réseau | Sortie DNS/HTTPS du VPS bloquée : rare chez OVH ; tester `server/peppol_resolver.py … --debug`. |
| `429` inattendus | Rate-limit de la clé trop bas — ajustez `rate`/`burst` de la clé dans le fichier. |
| Clone/`pull` en vrac après un échec | `sudo rm -rf /opt/superpopaul` puis relancez `install.sh` (idempotent). |
