# Ouvrir Super Popaul (binaires non signés)

## macOS
1. Dézippe `SuperPopaul-macos.zip`.
2. Double-clique sur `Super Popaul.app`. macOS refuse de l'ouvrir :
   c'est normal, l'app n'est pas signée. Ferme le message.
3. Ouvre **Réglages Système** → **Confidentialité et sécurité**.
4. Fais défiler jusqu'en bas : un message indique que « Super Popaul »
   a été bloqué.
5. Clique sur **Ouvrir quand même**, puis confirme.
   (Cette manipulation n'est nécessaire qu'une seule fois.)

Sur macOS 14 (Sonoma) et antérieurs, plus rapide : **clic droit** sur
`Super Popaul.app` → **Ouvrir** → bouton **Ouvrir**. (Depuis macOS 15
Sequoia, ce raccourci ne contourne plus le blocage : passe par les
Réglages comme ci-dessus.)

## Windows
1. Lance `Super Popaul.exe`.
2. Si SmartScreen affiche « Windows a protégé votre ordinateur » :
   **Informations complémentaires** → **Exécuter quand même**.
3. Prérequis : WebView2 (préinstallé sur Windows 10/11 récents). Si l'app
   ne démarre pas, installer « WebView2 Evergreen » depuis le site Microsoft.
