#!/usr/bin/env bash
# Empaqueta Resonda para Linux en dist/:
#   resonda-cli-<v>-linux-x86_64.tar.gz   la terminal, un solo binario
#   resonda_<v>_amd64.deb                  terminal + ventana (con Qt incluido, en /opt/resonda)
#   Resonda-<v>-x86_64.AppImage            la ventana, portable (con Qt incluido)
#
# Uso: packaging/build-linux.sh [--skip-build] [--no-appimage]
# Variables opcionales: QMAKE (ruta de qmake6), CMAKE_PREFIX_PATH (Qt fuera del sistema).
# Para que los paquetes funcionen en distros antiguas hay que compilar en una antigua
# (la CI de GitHub lo hace sobre Ubuntu 22.04).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DIST=$ROOT/dist STAGE=$ROOT/packaging/.stage CACHE=$ROOT/packaging/.cache
SKIP_BUILD=0 APPIMAGE=1
for a in "$@"; do case $a in --skip-build) SKIP_BUILD=1;; --no-appimage) APPIMAGE=0;; *) echo "opción desconocida: $a"; exit 2;; esac; done
say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

rm -rf "$DIST" "$STAGE"; mkdir -p "$DIST" "$STAGE" "$CACHE"

if [ $SKIP_BUILD = 0 ]; then
  say "Compilando la terminal"; cargo build --release
  say "Compilando la ventana"
  cmake -S gui -B gui/build -G Ninja -DCMAKE_BUILD_TYPE=Release ${CMAKE_PREFIX_PATH:+-DCMAKE_PREFIX_PATH="$CMAKE_PREFIX_PATH"}
  cmake --build gui/build
fi
CLI=target/release/resonda GUI=gui/build/resonda-gui
strip --strip-unneeded "$GUI" 2>/dev/null || true

# ---------- 1) .tar.gz de la terminal ----------
say "tar.gz (terminal)"
T=$STAGE/resonda-cli-$VERSION-linux-x86_64; mkdir -p "$T"
cp "$CLI" LICENSE README.md "$T/"
tar -C "$STAGE" --owner=0 --group=0 --numeric-owner -czf "$DIST/resonda-cli-$VERSION-linux-x86_64.tar.gz" "resonda-cli-$VERSION-linux-x86_64"

# ---------- 2) AppDir con la ventana y Qt (lo usan el AppImage y el .deb) ----------
APPDIR=$STAGE/AppDir
build_appdir() {
  say "Preparando AppDir (ventana + Qt)"
  local url_ld=https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
  local url_qt=https://github.com/linuxdeploy/linuxdeploy-plugin-qt/releases/download/continuous/linuxdeploy-plugin-qt-x86_64.AppImage
  for u in $url_ld $url_qt; do
    local f=$CACHE/$(basename "$u"); [ -x "$f" ] || { curl -fsSL -o "$f" "$u" && chmod +x "$f"; }
  done
  export APPIMAGE_EXTRACT_AND_RUN=1 NO_STRIP=1 ARCH=x86_64
  export QMAKE=${QMAKE:-$(command -v qmake6 || command -v qmake)}
  export QML_SOURCES_PATHS=$ROOT/gui/qml
  # Wayland: en Qt >= 6.5 el complemento es libqwayland.so; antes eran -generic y -egl.
  # Solo se piden los que existen en esta instalación de Qt.
  plug=$("$QMAKE" -query QT_INSTALL_PLUGINS)
  local extra="" f
  for f in libqwayland.so libqwayland-generic.so libqwayland-egl.so; do
    [ -f "$plug/platforms/$f" ] && extra="${extra:+$extra;}$f"
  done
  export EXTRA_PLATFORM_PLUGINS="$extra"
  export EXTRA_QT_MODULES="svg"
  [ -d "$plug/wayland-shell-integration" ] && EXTRA_QT_MODULES="svg;waylandclient"
  echo "  complementos de plataforma extra: ${EXTRA_PLATFORM_PLUGINS:-ninguno} | módulos extra: $EXTRA_QT_MODULES"
  rm -rf "$APPDIR"
  "$CACHE/linuxdeploy-x86_64.AppImage" --appdir "$APPDIR" \
    --executable "$GUI" --desktop-file gui/data/resonda.desktop --icon-file gui/data/resonda.svg \
    --icon-filename resonda --plugin qt
  # El módulo waylandclient no trae la integración gráfica (wayland-egl): sin ella Qt no puede crear
  # el contexto OpenGL en Wayland. Se copia y linuxdeploy añade sus dependencias.
  if [ -d "$plug/wayland-graphics-integration-client" ] && [ ! -d "$APPDIR/usr/plugins/wayland-graphics-integration-client" ]; then
    cp -a "$plug/wayland-graphics-integration-client" "$APPDIR/usr/plugins/"
    "$CACHE/linuxdeploy-x86_64.AppImage" --appdir "$APPDIR" --deploy-deps-only "$APPDIR/usr/plugins/wayland-graphics-integration-client"
  fi
  install -Dm755 "$CLI" "$APPDIR/usr/bin/resonda"
  install -Dm644 LICENSE "$APPDIR/usr/share/doc/resonda/copyright"
}

# ---------- 3) AppImage ----------
if [ $APPIMAGE = 1 ]; then
  build_appdir
  say "AppImage"
  url_at=https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  [ -x "$CACHE/appimagetool" ] || { curl -fsSL -o "$CACHE/appimagetool" "$url_at" && chmod +x "$CACHE/appimagetool"; }
  APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$CACHE/appimagetool" --no-appstream "$APPDIR" "$DIST/Resonda-$VERSION-x86_64.AppImage"
fi

# ---------- 4) .deb: terminal en /usr/bin y la ventana (con Qt) en /opt/resonda ----------
say ".deb"
D=$STAGE/deb; rm -rf "$D"; mkdir -p "$D/root/usr/bin" "$D/root/usr/share/doc/resonda" "$D/control"
install -m755 "$CLI" "$D/root/usr/bin/resonda"
install -m644 LICENSE "$D/root/usr/share/doc/resonda/copyright"
install -m644 README.md "$D/root/usr/share/doc/resonda/README.md"
if [ -d "$APPDIR" ]; then
  mkdir -p "$D/root/opt" "$D/root/usr/share/applications" "$D/root/usr/share/icons/hicolor/scalable/apps"
  cp -a "$APPDIR" "$D/root/opt/resonda"
  printf '#!/bin/sh\nexec /opt/resonda/AppRun "$@"\n' > "$D/root/usr/bin/resonda-gui"; chmod 755 "$D/root/usr/bin/resonda-gui"
  install -m644 gui/data/resonda.desktop "$D/root/usr/share/applications/resonda.desktop"
  install -m644 gui/data/resonda.svg "$D/root/usr/share/icons/hicolor/scalable/apps/resonda.svg"
fi
SIZE=$(du -sk "$D/root" | cut -f1)
cat > "$D/control/control" <<CTL
Package: resonda
Version: $VERSION
Section: sound
Priority: optional
Architecture: amd64
Installed-Size: $SIZE
Depends: libc6 (>= 2.31), libgl1, libegl1
Maintainer: Resonda contributors <noreply@github.com>
Description: Descarga vídeos y música de YouTube y Spotify
 Resonda descarga vídeos y música de YouTube y Spotify a MP4, MP3, FLAC, WAV,
 M4A o AAC. Código propio y libre (MIT): sin yt-dlp ni ffmpeg.
 Incluye la terminal (resonda) y la ventana (resonda-gui).
CTL
( cd "$D/root" && find . -type f -exec md5sum {} + | sed 's|\./||' > "$D/control/md5sums" )
tar -C "$D/control" --owner=0 --group=0 --numeric-owner -cJf "$D/control.tar.xz" .
tar -C "$D/root" --owner=0 --group=0 --numeric-owner -cJf "$D/data.tar.xz" .
printf '2.0\n' > "$D/debian-binary"
( cd "$D" && ar rc "$DIST/resonda_${VERSION}_amd64.deb" debian-binary control.tar.xz data.tar.xz )

say "Listo"; ls -la "$DIST" | awk 'NR>1 {printf "  %-46s %8.1f MB\n", $9, $5/1048576}'
( cd "$DIST" && sha256sum * > SHA256SUMS.txt && echo "  SHA256SUMS.txt" )
