#include "backend.h"

#include <QDateTime>
#include <QDesktopServices>
#include <QFileInfo>
#include <QMetaObject>
#include <QRegularExpression>
#include <QSettings>
#include <QStandardPaths>
#include <QThread>

namespace {
QString clock(quint64 s)
{
    return s >= 3600 ? QStringLiteral("%1:%2:%3").arg(s / 3600).arg(s / 60 % 60, 2, 10, QChar('0')).arg(s % 60, 2, 10, QChar('0'))
                     : QStringLiteral("%1:%2").arg(s / 60).arg(s % 60, 2, 10, QChar('0'));
}
QString takeString(char *s)
{
    const QString r = s ? QString::fromUtf8(s) : QString();
    rs_string_free(s);
    return r;
}
}

Backend::Backend(QObject *parent)
    : QObject(parent), m_cancel(rs_cancel_new())
{
    QSettings settings;
    m_setupDone = settings.value(QStringLiteral("setupDone"), false).toBool();
    m_services = settings.value(QStringLiteral("services"), QStringList{QStringLiteral("youtube"), QStringLiteral("spotify")}).toStringList();
}

void Backend::saveServices(const QStringList &ids)
{
    m_services = ids;
    m_setupDone = true;
    QSettings settings;
    settings.setValue(QStringLiteral("services"), m_services);
    settings.setValue(QStringLiteral("setupDone"), true);
    emit servicesChanged();
}

Backend::~Backend()
{
    // Si hay una descarga en marcha se cancela y se espera a que el hilo termine
    rs_cancel_set(m_cancel);
    if (m_worker) {
        m_worker->wait();
        delete m_worker;
    }
    rs_info_free(m_info);
    rs_spotify_free(m_spotify);
    rs_cancel_free(m_cancel);
}

QUrl Backend::defaultFolder() const
{
    QString dir = QStandardPaths::writableLocation(QStandardPaths::DownloadLocation);
    if (dir.isEmpty())
        dir = QStandardPaths::writableLocation(QStandardPaths::HomeLocation);
    return QUrl::fromLocalFile(dir);
}

void Backend::runWorker(std::function<void()> fn)
{
    if (m_worker) {
        m_worker->wait(); // el anterior ya terminó su trabajo; esto es inmediato
        delete m_worker;
    }
    m_worker = QThread::create(std::move(fn));
    m_worker->start();
}

// El vídeo o la lista anteriores dejan de corresponder a lo que se busca: se descartan ya, así un
// fallo no los deja en pantalla junto al error. (No hay descarga en curso: se comprueba antes.)
void Backend::clearSource()
{
    rs_info_free(m_info);
    m_info = nullptr;
    rs_spotify_free(m_spotify);
    m_spotify = nullptr;
    m_heights.clear();
    m_tracks.clear();
    emit videoChanged();
}

// ---------- consultar una fuente ----------

void Backend::fetchInfo(const QString &text)
{
    if (m_fetching || m_downloading)
        return;
    const QByteArray input = text.trimmed().toUtf8();
    if (input.isEmpty()) {
        setError(tr("Pega la URL o el ID de un vídeo de YouTube, o un enlace de Spotify."));
        return;
    }
    const bool isSpotify = rs_is_spotify_link(input.constData());
    const QString service = isSpotify ? QStringLiteral("spotify") : QStringLiteral("youtube");
    if (!m_services.contains(service)) {
        setError(tr("%1 está desactivado. Actívalo en «Servicios» (arriba a la derecha).")
                     .arg(isSpotify ? QStringLiteral("Spotify") : QStringLiteral("YouTube")));
        return;
    }
    clearSource();
    setError({});
    setResult({}, {});
    setProgress(0);
    setFetching(true);

    if (isSpotify) {
        setStatus(tr("Leyendo Spotify…"));
        runWorker([this, input] {
            char *err = nullptr;
            RsSpotify *s = rs_spotify_fetch(input.constData(), &err);
            const QString errText = takeString(err);
            QMetaObject::invokeMethod(this, [this, s, errText] { finishFetchSpotify(s, errText); }, Qt::QueuedConnection);
        });
        return;
    }

    setStatus(tr("Buscando vídeo…"));
    runWorker([this, input] {
        char *err = nullptr;
        RsInfo *info = rs_info_fetch(input.constData(), &err);
        const QString errText = takeString(err);
        QMetaObject::invokeMethod(this, [this, info, errText] { finishFetch(info, errText); }, Qt::QueuedConnection);
    });
}

void Backend::finishFetch(RsInfo *info, const QString &error)
{
    setFetching(false);
    setStatus({});
    if (!info) {
        setError(error);
        return;
    }
    m_info = info;

    m_title = QString::fromUtf8(rs_info_title(info));
    m_thumbnail = QStringLiteral("https://i.ytimg.com/vi/%1/hqdefault.jpg").arg(QString::fromUtf8(rs_info_id(info)));
    m_duration = clock(rs_info_seconds(info));

    uint32_t buf[64];
    const size_t n = qMin<size_t>(rs_info_heights(info, buf, 64), 64);
    for (size_t i = 0; i < n; ++i)
        m_heights.append(int(buf[i]));
    emit videoChanged();
}

void Backend::finishFetchSpotify(RsSpotify *s, const QString &error)
{
    setFetching(false);
    setStatus({});
    if (!s) {
        setError(error);
        return;
    }
    m_spotify = s;

    const size_t n = rs_spotify_count(s);
    m_title = QString::fromUtf8(rs_spotify_name(s));
    m_thumbnail = QString::fromUtf8(rs_spotify_cover(s));
    for (size_t i = 0; i < n; ++i)
        m_tracks.append(QStringLiteral("%1  ·  %2").arg(QString::fromUtf8(rs_spotify_track_label(s, i)), clock(rs_spotify_track_seconds(s, i))));
    m_duration = n == 1 ? clock(rs_spotify_track_seconds(s, 0)) : tr("%1 canciones").arg(n);
    emit videoChanged();
}

// ---------- descargar ----------

void Backend::download(int maxHeight, int format, int bitrate, const QUrl &folder)
{
    if (!hasVideo() || m_fetching || m_downloading)
        return;
    QString dir = folder.isLocalFile() ? folder.toLocalFile() : folder.toString();
    if (dir.isEmpty())
        dir = defaultFolder().toLocalFile();

    m_lastUiUpdate = 0;
    rs_cancel_reset(m_cancel);
    setError({});
    setResult({}, {});
    setProgress(0);
    setDownloading(true);

    if (m_spotify)
        downloadSpotify(dir.toUtf8(), format, bitrate);
    else
        downloadYoutube(maxHeight, format, bitrate, dir.toUtf8());
}

void Backend::downloadYoutube(int maxHeight, int format, int bitrate, const QByteArray &dir)
{
    m_audioOnly = format != RS_FORMAT_MP4;
    m_trackTotal = 0; // 0 = no es Spotify
    setStatus(m_audioOnly ? tr("Descargando audio…") : tr("Descargando vídeo…"));

    RsInfo *info = m_info;
    const uint32_t height = m_audioOnly ? 0 : uint32_t(qMax(maxHeight, 0));
    runWorker([this, info, dir, height, format, bitrate] {
        RsOptions opts{height, format, uint32_t(qMax(bitrate, 0)), dir.constData()};
        char *path = nullptr;
        char *err = nullptr;
        const int code = rs_download(info, &opts, &Backend::progressCallback, this, m_cancel, &path, &err);
        const QString pathText = takeString(path);
        const QString errText = takeString(err);
        QMetaObject::invokeMethod(this, [this, code, pathText, errText] { finishDownload(code, pathText, errText); },
                                  Qt::QueuedConnection);
    });
}

// Una canción tras otra; el progreso es el de toda la lista
void Backend::downloadSpotify(const QByteArray &dir, int format, int bitrate)
{
    RsSpotify *s = m_spotify;
    const size_t n = rs_spotify_count(s);
    m_audioOnly = true;
    m_trackTotal = int(n);
    m_currentTrack = 0;
    // Álbumes y playlists van a su propia carpeta
    const bool single = rs_spotify_kind(s) == 0;
    QString target = QString::fromUtf8(dir);
    if (!single) {
        QString folderName = m_title;
        folderName.replace(QRegularExpression(QStringLiteral("[\\\\/:*?\"<>|]")), QStringLiteral("_"));
        target += QStringLiteral("/") + folderName.trimmed();
    }
    const QByteArray targetUtf8 = target.toUtf8();

    runWorker([this, s, n, targetUtf8, format, bitrate] {
        int ok = 0, failed = 0;
        bool cancelled = false;
        QString lastPath;
        QStringList errors;
        for (size_t i = 0; i < n; ++i) {
            m_currentTrack = int(i);
            const QString label = QString::fromUtf8(rs_spotify_track_label(s, i));
            QMetaObject::invokeMethod(this, [this, i, n, label] {
                m_lastUiUpdate = 0;
                setStatus(tr("Canción %1 de %2 · %3").arg(i + 1).arg(n).arg(label));
                setProgress(double(i) / double(n), {});
            }, Qt::QueuedConnection);

            char *path = nullptr, *matched = nullptr, *err = nullptr;
            const int code = rs_spotify_download(s, i, targetUtf8.constData(), format, uint32_t(qMax(bitrate, 0)), &Backend::progressCallback, this, m_cancel,
                                                 &path, &matched, &err);
            const QString pathText = takeString(path);
            takeString(matched);
            const QString errText = takeString(err);
            if (code == RS_OK) {
                ++ok;
                lastPath = pathText;
            } else if (code == RS_CANCELLED) {
                cancelled = true;
                break;
            } else {
                ++failed;
                errors << QStringLiteral("%1: %2").arg(label, errText);
            }
        }
        QMetaObject::invokeMethod(this, [this, ok, failed, cancelled, lastPath, errors] {
            finishSpotifyDownload(ok, failed, cancelled, lastPath, errors);
        }, Qt::QueuedConnection);
    });
}

// Se llama desde los hilos de descarga del núcleo: solo reenvía (limitando la frecuencia).
void Backend::progressCallback(int stage, uint64_t done, uint64_t total, void *user)
{
    auto *self = static_cast<Backend *>(user);
    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    if (done < total && now - self->m_lastUiUpdate.load() < 80)
        return;
    self->m_lastUiUpdate = now;
    QMetaObject::invokeMethod(self, [self, stage, done, total] { self->onProgress(stage, done, total); },
                              Qt::QueuedConnection);
}

void Backend::onProgress(int stage, uint64_t done, uint64_t total)
{
    if (!m_downloading)
        return; // llegó tarde, ya terminó o se canceló
    const bool list = m_trackTotal > 0; // Spotify: el estado ya dice qué canción es

    if (stage == RS_STAGE_MUXING) {
        if (!list) {
            setStatus(tr("Uniendo audio y vídeo…"));
            setProgress(-1);
        }
        return;
    }
    const double mb = 1024.0 * 1024.0;
    const double frac = total ? double(done) / double(total) : 0;
    if (list) {
        setProgress((m_currentTrack.load() + frac) / double(m_trackTotal),
                    tr("%1 de %2 MB").arg(done / mb, 0, 'f', 1).arg(total / mb, 0, 'f', 1));
        return;
    }
    setStatus(stage == RS_STAGE_AUDIO || m_audioOnly ? tr("Descargando audio…") : tr("Descargando vídeo…"));
    setProgress(frac, tr("%1 de %2 MB").arg(done / mb, 0, 'f', 1).arg(total / mb, 0, 'f', 1));
}

void Backend::finishDownload(int code, const QString &path, const QString &error)
{
    setDownloading(false);
    if (code == RS_OK) {
        setProgress(1);
        setStatus(tr("Descarga completada"));
        setResult(path, QFileInfo(path).fileName());
    } else if (code == RS_CANCELLED) {
        setProgress(0);
        setStatus(tr("Descarga cancelada"));
    } else {
        setProgress(0);
        setStatus({});
        setError(error);
    }
}

void Backend::finishSpotifyDownload(int ok, int failed, bool cancelled, const QString &lastPath, const QStringList &errors)
{
    setDownloading(false);
    if (cancelled) {
        setProgress(0);
        setStatus(tr("Descarga cancelada"));
    } else {
        setProgress(ok > 0 ? 1 : 0);
        setStatus(ok > 0 ? tr("Descarga completada") : QString());
    }
    if (ok > 0) {
        QString text = tr("%n canción(es) guardada(s)", nullptr, ok);
        if (failed > 0)
            text += tr(" · %n no se pudo(ieron) descargar", nullptr, failed);
        setResult(lastPath, text);
    }
    if (!errors.isEmpty())
        setError(errors.join(QLatin1Char('\n')));
}

void Backend::cancel()
{
    if (!m_downloading)
        return;
    rs_cancel_set(m_cancel);
    setStatus(tr("Cancelando…"));
}

void Backend::showInFolder(const QString &path)
{
    QDesktopServices::openUrl(QUrl::fromLocalFile(QFileInfo(path).absolutePath()));
}

// ---------- setters ----------

void Backend::setFetching(bool v) { if (m_fetching != v) { m_fetching = v; emit fetchingChanged(); } }
void Backend::setDownloading(bool v) { if (m_downloading != v) { m_downloading = v; emit downloadingChanged(); } }
void Backend::setStatus(const QString &v) { if (m_status != v) { m_status = v; emit statusChanged(); } }
void Backend::setError(const QString &v) { if (m_error != v) { m_error = v; emit errorChanged(); } }
void Backend::setResult(const QString &path, const QString &text)
{
    if (m_resultPath != path || m_resultText != text) {
        m_resultPath = path;
        m_resultText = text;
        emit resultChanged();
    }
}
void Backend::setProgress(double v, const QString &text)
{
    m_progress = v;
    m_progressText = text;
    emit progressChanged();
}
