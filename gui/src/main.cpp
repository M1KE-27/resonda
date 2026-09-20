#include <QtGlobal>
#include <QDir>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QStandardPaths>

#ifdef Q_OS_UNIX
#  include <csignal>
#  include <execinfo.h>
#  include <fcntl.h>
#  include <unistd.h>
#endif
// Si la app muere por una señal fatal (SIGSEGV, SIGABRT...), deja constancia en
// <datos de la app>/crash.log con la señal y la traza. Solo usa llamadas seguras en un manejador
// de señales (write, backtrace_symbols_fd).
#ifdef Q_OS_UNIX
static int g_crashFd = -1;

static void onFatalSignal(int sig)
{
    if (g_crashFd >= 0) {
        static const char head[] = "\n=== Resonda terminó por una señal fatal: ";
        (void)!write(g_crashFd, head, sizeof head - 1);
        char digits[12];
        int n = 0;
        for (int v = sig; v > 0 || n == 0; v /= 10)
            digits[n++] = char('0' + v % 10);
        for (int i = n - 1; i >= 0; --i)
            (void)!write(g_crashFd, &digits[i], 1);
        (void)!write(g_crashFd, "\n", 1);
        void *frames[48];
        backtrace_symbols_fd(frames, backtrace(frames, 48), g_crashFd);
    }
    signal(sig, SIG_DFL);
    raise(sig);
}

static void installCrashLog()
{
    const QString dir = QStandardPaths::writableLocation(QStandardPaths::AppLocalDataLocation);
    QDir().mkpath(dir);
    g_crashFd = open(QFile::encodeName(dir + "/crash.log").constData(), O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0644);
    for (int sig : {SIGSEGV, SIGABRT, SIGBUS, SIGFPE, SIGILL}) {
        struct sigaction sa {};
        sa.sa_handler = onFatalSignal;
        sigemptyset(&sa.sa_mask);
        sigaction(sig, &sa, nullptr);
    }
}
#else
static void installCrashLog() {} // el registro de señales fatales es solo para POSIX
#endif

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    app.setOrganizationName("Resonda");
    app.setApplicationName("Resonda");
    app.setDesktopFileName("resonda");
    installCrashLog();

    // Mismo aspecto en cualquier escritorio (KDE, GNOME, Hyprland...)
    QQuickStyle::setStyle("Material");

    // resonda-gui [URL]: abre la app con ese vídeo ya cargado
    const QStringList args = app.arguments();

    QQmlApplicationEngine engine;
    engine.setInitialProperties({{"initialUrl", args.size() > 1 ? args.at(1) : QString()}});
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app,
                     [] { QCoreApplication::exit(-1); }, Qt::QueuedConnection);
    engine.loadFromModule("Resonda", "Main");
    return app.exec();
}
