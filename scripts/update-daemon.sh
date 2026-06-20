#!/usr/bin/env bash
set -e

# Este script automatiza o rebuild e o recarregamento do daemon durante o desenvolvimento

echo "🚀 Compilando o ACPD em modo Release..."
~/.cargo/bin/cargo build --release

echo "🔄 Reiniciando o serviço no Systemd..."
systemctl --user daemon-reload
systemctl --user restart acpd

echo "✅ Pronto! O ACPD está rodando com o seu código mais recente."
systemctl --user status acpd --no-pager
