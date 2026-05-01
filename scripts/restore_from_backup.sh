#!/bin/sh
# Disaster recovery: Yedekten otomatik geri yükleme scripti
# K8s podunda veya localde çalıştırılabilir

BACKUP_FILE=$1
if [ -z "$BACKUP_FILE" ]; then
  echo "Kullanım: $0 <backup-file.tar.gz>"; exit 1
fi

tar xzf "$BACKUP_FILE" -C /
echo "Yedek başarıyla geri yüklendi."
