#!/bin/bash
# Memos Trading - Bash Aliases
# Aşağıdaki satırı ~/.bashrc veya ~/.bash_profile'a ekleyin:
# source /home/ulas/PyCharmMiscProject/memos_trading/.bash_aliases

MEMOS_ROOT="/home/ulas/PyCharmMiscProject/memos_trading"

# Ana menü
alias memos_menu="cd $MEMOS_ROOT && ./run_menu.sh"
alias memos_quick="cd $MEMOS_ROOT && bash quick_start.sh"

# Dev mode
alias memos_dev="cd $MEMOS_ROOT && ./run_menu.sh << EOF
6
EOF"

# Build
alias memos_build="cd $MEMOS_ROOT && ./run_menu.sh << EOF
9
0
EOF"

# Release
alias memos_release="cd $MEMOS_ROOT && ./run_menu.sh << EOF
7
EOF"

# Test
alias memos_test="cd $MEMOS_ROOT && ./run_menu.sh << EOF
8
0
EOF"

# Status
alias memos_status="cd $MEMOS_ROOT && ./run_menu.sh << EOF
11
0
EOF"

# Clean
alias memos_clean="cd $MEMOS_ROOT && ./run_menu.sh << EOF
10
0
EOF"

# Clippy
alias memos_check="cd $MEMOS_ROOT && ./run_menu.sh << EOF
12
0
EOF"

# Full clean
alias memos_fullclean="cd $MEMOS_ROOT && ./run_menu.sh << EOF
13
y
10
0
EOF"

echo "✅ Memos Trading aliases yüklendi!"
echo "Örnekler:"
echo "  memos_menu       - Ana menü açıyken (interaktif)"
echo "  memos_quick      - Hızlı başlangıç (interaktif)"
echo "  memos_dev        - Dev mode başlat"
echo "  memos_build      - Production build"
echo "  memos_release    - Release çalıştır"
echo "  memos_test       - Test'leri çalıştır"
echo "  memos_status     - Durum kontrolü"
echo "  memos_check      - Clippy analizi"
