#pragma once

#include <QObject>
#include <QStringList>
#include <QUrl>
#include <QVariantList>
#include <QtQml/qqml.h>
#include <atomic>
#include <functional>

#include "resonda.h"

class QThread;

// Puente entre la interfaz QML y el núcleo Rust (API C de resonda.h).
// Las llamadas al núcleo bloquean, así que se hacen en un hilo de trabajo y los
// resultados vuelven al hilo de la interfaz mediante invocaciones en cola.
//
// La fuente puede ser un vídeo de YouTube o un enlace de Spotify (canción, álbum o playlist):
// de Spotify solo se leen los datos y cada canción se busca y se descarga desde YouTube.
class Backend : public QObject
{
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(bool fetching READ fetching NOTIFY fetchingChanged)
    Q_PROPERTY(bool downloading READ downloading NOTIFY downloadingChanged)
    Q_PROPERTY(bool hasVideo READ hasVideo NOTIFY videoChanged) // hay una fuente cargada (YouTube o Spotify)
    Q_PROPERTY(bool spotify READ spotify NOTIFY videoChanged)
    Q_PROPERTY(QString title READ title NOTIFY videoChanged)
    Q_PROPERTY(QString thumbnailUrl READ thumbnailUrl NOTIFY videoChanged)
    Q_PROPERTY(QString duration READ duration NOTIFY videoChanged)
    Q_PROPERTY(QVariantList heights READ heights NOTIFY videoChanged)
    Q_PROPERTY(QStringList tracks READ tracks NOTIFY videoChanged) // canciones de Spotify: "Artista - Título · 3:33"
    Q_PROPERTY(QString status READ status NOTIFY statusChanged)
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)
    Q_PROPERTY(double progress READ progress NOTIFY progressChanged) // 0..1; -1 = indeterminado
    Q_PROPERTY(QString progressText READ progressText NOTIFY progressChanged)
    Q_PROPERTY(QString resultPath READ resultPath NOTIFY resultChanged)
    Q_PROPERTY(QString resultText READ resultText NOTIFY resultChanged)
    Q_PROPERTY(QUrl defaultFolder READ defaultFolder CONSTANT)
    Q_PROPERTY(QStringList services READ services NOTIFY servicesChanged) // servicios activados
    Q_PROPERTY(bool setupDone READ setupDone NOTIFY servicesChanged)      // ya se eligieron los servicios

public:
    explicit Backend(QObject *parent = nullptr);
    ~Backend() override;

    // Acepta una URL/ID de YouTube o un enlace de Spotify
    Q_INVOKABLE void fetchInfo(const QString &input);
    // maxHeight solo se usa con vídeo de YouTube; format es RS_FORMAT_*; bitrate (kbps) solo con MP3
    Q_INVOKABLE void download(int maxHeight, int format, int bitrate, const QUrl &folder);
    // Guarda qué servicios quiere usar la persona (se pregunta la primera vez)
    Q_INVOKABLE void saveServices(const QStringList &ids);
    Q_INVOKABLE void cancel();
    Q_INVOKABLE void showInFolder(const QString &path);

    bool fetching() const { return m_fetching; }
    bool downloading() const { return m_downloading; }
    bool hasVideo() const { return m_info != nullptr || m_spotify != nullptr; }
    bool spotify() const { return m_spotify != nullptr; }
    QString title() const { return m_title; }
    QString thumbnailUrl() const { return m_thumbnail; }
    QString duration() const { return m_duration; }
    QVariantList heights() const { return m_heights; }
    QStringList tracks() const { return m_tracks; }
    QString status() const { return m_status; }
    QString error() const { return m_error; }
    double progress() const { return m_progress; }
    QString progressText() const { return m_progressText; }
    QString resultPath() const { return m_resultPath; }
    QString resultText() const { return m_resultText; }
    QUrl defaultFolder() const;
    QStringList services() const { return m_services; }
    bool setupDone() const { return m_setupDone; }

signals:
    void fetchingChanged();
    void downloadingChanged();
    void videoChanged();
    void statusChanged();
    void errorChanged();
    void progressChanged();
    void resultChanged();
    void servicesChanged();

private:
    static void progressCallback(int stage, uint64_t done, uint64_t total, void *user);
    void onProgress(int stage, uint64_t done, uint64_t total);
    void clearSource();
    void finishFetch(RsInfo *info, const QString &error);
    void finishFetchSpotify(RsSpotify *spotify, const QString &error);
    void finishDownload(int code, const QString &path, const QString &error);
    void finishSpotifyDownload(int ok, int failed, bool cancelled, const QString &lastPath, const QStringList &errors);
    void downloadYoutube(int maxHeight, int format, int bitrate, const QByteArray &dir);
    void downloadSpotify(const QByteArray &dir, int format, int bitrate);
    void runWorker(std::function<void()> fn);

    void setFetching(bool v);
    void setDownloading(bool v);
    void setStatus(const QString &v);
    void setError(const QString &v);
    void setProgress(double v, const QString &text = {});
    void setResult(const QString &path, const QString &text);

    RsInfo *m_info = nullptr;
    RsSpotify *m_spotify = nullptr;
    RsCancel *m_cancel = nullptr;
    QThread *m_worker = nullptr;
    std::atomic<qint64> m_lastUiUpdate{0};
    std::atomic<int> m_currentTrack{0}; // canción de Spotify en curso (para el progreso global)
    int m_trackTotal = 1;

    bool m_fetching = false;
    bool m_downloading = false;
    bool m_audioOnly = false;
    QString m_title, m_thumbnail, m_duration, m_status, m_error, m_progressText, m_resultPath, m_resultText;
    QVariantList m_heights;
    QStringList m_tracks;
    QStringList m_services;
    bool m_setupDone = false;
    double m_progress = 0;
};
