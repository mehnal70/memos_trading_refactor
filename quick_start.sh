#!/bin/bash

# Memos Trading - Quick Start Guide
# İlk başlayanlar için hızlı başlangıç rehberi

echo "======================================================"
echo "    🚀 MEMOS TRADING - HIZLI BAŞLANGIÇ REHBERİ"
echo "======================================================"
echo ""

# Seçenekleri göster
echo "Yapmak istediğiniz işlemi seçin:"
echo ""
echo "🎯 TEMEL (İlk Defa Kurulum İçin)"
echo "  1) ⚙️  İlk Defa Kurulum (Full Setup)"
echo "  2) 🚀 Hemen Başlat (Dev Mode)"
echo ""
echo "📦 SÜRÜM HAZIRLA"
echo "  3) 📦 Production Build Yap"
echo "  4) 📍 Release Binary'yi Çalıştır"
echo ""
echo "🧹 BAKIŞ & TEMIZLIK"
echo "  5) 🔍 Sistem Kontrolü Yap"
echo "  6) 🧹 Tümünü Temizle & Yeniden Kurulum"
echo ""
echo "0) ❌ İptal"
echo ""

read -p "Seçiminiz (0-6): " choice

case $choice in
    1)
        echo ""
        echo "📦 İlk Defa Kurulum başlatılıyor..."
        echo ""
        read -p "npm bağımlılıklarını yüklemek ister misiniz? (y/n) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            cd memos_trading_desktop
            npm install
            cd ..
        fi
        echo "🔨 Full Build başlatılıyor..."
        bash run_menu.sh << EOF
9
0
EOF
        echo ""
        echo "✅ Kurulum tamamlandı! Seçenek 2 ile başlat."
        ;;
    2)
        echo ""
        echo "🚀 Dev Mode başlatılıyor..."
        echo "💡 Tarayıcı otomatik açılacak. http://localhost:5173"
        echo ""
        bash run_menu.sh << EOF
6
EOF
        ;;
    3)
        echo ""
        echo "📦 Production Build başlatılıyor..."
        bash run_menu.sh << EOF
9
0
EOF
        echo ""
        echo "✅ Build tamamlandı!"
        echo "📍 Binary: memos_trading_desktop/src-tauri/target/release/memos_trading_desktop"
        ;;
    4)
        echo ""
        echo "📍 Release Binary çalıştırılıyor..."
        bash run_menu.sh << EOF
7
EOF
        ;;
    5)
        echo ""
        echo "🔍 Sistem kontrol yapılıyor..."
        bash run_menu.sh << EOF
11
0
EOF
        ;;
    6)
        echo ""
        echo "🧹 Tümü temizleniyor..."
        bash run_menu.sh << EOF
13
y
10
0
EOF
        echo ""
        echo "✅ Temizlik tamamlandı!"
        echo "🔨 Şimdi seçenek 1 ile kurulum yapabilirsiniz."
        ;;
    0)
        echo "İptal edildi."
        exit 0
        ;;
    *)
        echo "❌ Geçersiz seçim!"
        exit 1
        ;;
esac

echo ""
read -p "Başka işlem yapmak ister misiniz? (y/n) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    bash quick_start.sh
fi
