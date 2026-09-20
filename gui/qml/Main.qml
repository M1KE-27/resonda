import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Dialogs
import QtQuick.Layouts
import Resonda

ApplicationWindow {
    id: win

    width: 780
    height: 860
    minimumWidth: 540
    minimumHeight: 560
    visible: true
    title: "Resonda"

    Material.theme: Material.Dark
    Material.accent: "#ff5a4f"

    property string initialUrl: ""
    property url folder: backend.defaultFolder
    // La pantalla de servicios aparece la primera vez y cuando se pulsa «Servicios»
    property bool setupOpen: !backend.setupDone
    // Formato elegido: mismos valores que RS_FORMAT_* de la API C
    property int format: 0
    property int mp3kbps: 192

    readonly property bool idle: !backend.fetching && !backend.downloading
    readonly property bool videoFormat: format === 0
    readonly property int chosenHeight: qualityBox.currentIndex >= 0 && backend.heights.length > 0
                                        ? backend.heights[qualityBox.currentIndex] : 0

    readonly property var allFormats: [
        { value: 0, label: "MP4",  hint: qsTr("Vídeo con audio") },
        { value: 5, label: "MP3",  hint: qsTr("Audio ligero, suena en cualquier sitio") },
        { value: 4, label: "FLAC", hint: qsTr("Audio sin pérdida (archivos más grandes)") },
        { value: 3, label: "WAV",  hint: qsTr("Audio sin comprimir") },
        { value: 1, label: "M4A",  hint: qsTr("Audio AAC en MP4, sin recodificar") },
        { value: 2, label: "AAC",  hint: qsTr("Audio AAC crudo, sin recodificar") }
    ]
    // Spotify es solo música: sin vídeo
    readonly property var formatChoices: backend.spotify ? allFormats.slice(1) : allFormats
    readonly property string formatHint: {
        for (const f of allFormats)
            if (f.value === format)
                return f.hint
        return ""
    }
    readonly property string placeholder: {
        const names = []
        if (backend.services.indexOf("youtube") >= 0) names.push("YouTube")
        if (backend.services.indexOf("spotify") >= 0) names.push("Spotify")
        return names.length ? qsTr("Pega un enlace de %1").arg(names.join(qsTr(" o de "))) : qsTr("Activa un servicio en «Servicios»")
    }

    Backend {
        id: backend
        // La música se baja por defecto en MP3
        onVideoChanged: if (backend.spotify && win.format === 0) win.format = 5
    }

    FolderDialog {
        id: folderDialog
        currentFolder: win.folder
        onAccepted: win.folder = selectedFolder
    }

    Component.onCompleted: {
        if (initialUrl !== "" && !setupOpen) {
            urlField.text = initialUrl
            search()
        }
    }

    function search() {
        if (urlField.text.trim().length > 0 && win.idle)
            backend.fetchInfo(urlField.text)
    }

    // Por defecto 1080p (H.264 reproduce en todas partes); por encima solo hay AV1.
    // Las alturas vienen de mayor a menor: se toma la primera que no pase de 1080.
    function defaultQualityIndex() {
        for (let i = 0; i < backend.heights.length; ++i) {
            if (backend.heights[i] <= 1080)
                return i
        }
        return 0
    }

    function fileName(path) {
        return path.substring(path.lastIndexOf("/") + 1)
    }

    // Botón "píldora" para elegir una opción (formato, calidad...)
    component Chip: AbstractButton {
        id: chip
        property bool selected: false
        implicitHeight: 38
        leftPadding: 18
        rightPadding: 18
        background: Rectangle {
            radius: height / 2
            color: chip.selected ? Material.accent : (chip.hovered ? "#2a2a30" : "transparent")
            border.width: 1
            border.color: chip.selected ? Material.accent : "#4dffffff"
        }
        contentItem: Label {
            text: chip.text
            color: chip.selected ? "white" : Material.foreground
            font.weight: Font.Medium
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            opacity: chip.enabled ? 1 : 0.5
        }
    }

    // ---------------- pantalla de servicios ----------------
    ServiceSetup {
        anchors.fill: parent
        visible: win.setupOpen
        firstRun: !backend.setupDone
        initial: backend.services
        onAccepted: ids => {
            backend.saveServices(ids)
            win.setupOpen = false
        }
        onCancelled: win.setupOpen = false
    }

    // ---------------- pantalla principal ----------------
    ScrollView {
        id: scroll
        anchors.fill: parent
        visible: !win.setupOpen
        contentWidth: availableWidth
        clip: true

        ColumnLayout {
            width: scroll.availableWidth
            spacing: 0

            ColumnLayout {
                Layout.fillWidth: true
                Layout.margins: 28
                spacing: 18

                // ---- Cabecera ----
                RowLayout {
                    Layout.fillWidth: true
                    ColumnLayout {
                        spacing: 2
                        Label {
                            text: "Resonda"
                            font.pixelSize: 30
                            font.weight: Font.Bold
                        }
                        Label {
                            text: qsTr("Descarga vídeos y música")
                            opacity: 0.6
                        }
                    }
                    Item { Layout.fillWidth: true }
                    Button {
                        flat: true
                        text: "⚙  " + qsTr("Servicios")
                        enabled: win.idle
                        onClicked: win.setupOpen = true
                    }
                }

                // ---- Buscar ----
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    TextField {
                        id: urlField
                        Layout.fillWidth: true
                        placeholderText: win.placeholder
                        selectByMouse: true
                        enabled: !backend.downloading
                        onAccepted: win.search()
                    }
                    Button {
                        text: backend.fetching ? qsTr("Buscando…") : qsTr("Buscar")
                        highlighted: !backend.hasVideo
                        enabled: win.idle && urlField.text.trim().length > 0
                        onClicked: win.search()
                    }
                }

                // ---- Error ----
                Pane {
                    Layout.fillWidth: true
                    visible: backend.error !== ""
                    Material.background: "#4a1f1f"
                    Material.elevation: 0
                    padding: 14

                    Label {
                        width: parent.width
                        text: backend.error
                        wrapMode: Text.Wrap
                        color: "#ffb4ab"
                    }
                }

                BusyIndicator {
                    Layout.alignment: Qt.AlignHCenter
                    running: backend.fetching
                    visible: backend.fetching
                }

                // ---- Fuente: vídeo o lista de Spotify ----
                Pane {
                    Layout.fillWidth: true
                    visible: backend.hasVideo
                    Material.elevation: 2
                    padding: 16

                    RowLayout {
                        width: parent.width
                        spacing: 18

                        Rectangle {
                            Layout.preferredWidth: 224
                            Layout.preferredHeight: 126
                            radius: 8
                            color: "#1b1b1b"
                            clip: true

                            Image {
                                anchors.fill: parent
                                source: backend.thumbnailUrl
                                fillMode: Image.PreserveAspectCrop
                                asynchronous: true
                            }
                            Rectangle {
                                anchors { right: parent.right; bottom: parent.bottom; margins: 6 }
                                width: durationLabel.implicitWidth + 12
                                height: durationLabel.implicitHeight + 4
                                radius: 4
                                color: "#cc000000"
                                Label {
                                    id: durationLabel
                                    anchors.centerIn: parent
                                    text: backend.duration
                                    font.pixelSize: 12
                                    color: "white"
                                }
                            }
                        }

                        Label {
                            Layout.fillWidth: true
                            Layout.alignment: Qt.AlignTop
                            text: backend.title
                            font.pixelSize: 17
                            font.weight: Font.Medium
                            wrapMode: Text.Wrap
                            maximumLineCount: 4
                            elide: Text.ElideRight
                        }
                    }
                }

                // ---- Formato ----
                Pane {
                    Layout.fillWidth: true
                    visible: backend.hasVideo
                    Material.elevation: 1
                    padding: 18

                    ColumnLayout {
                        width: parent.width
                        spacing: 14

                        Label {
                            text: qsTr("FORMATO")
                            font.pixelSize: 12
                            font.weight: Font.Medium
                            font.letterSpacing: 1
                            opacity: 0.55
                        }
                        Flow {
                            Layout.fillWidth: true
                            spacing: 8
                            Repeater {
                                model: win.formatChoices
                                delegate: Chip {
                                    text: modelData.label
                                    selected: win.format === modelData.value
                                    enabled: win.idle
                                    onClicked: win.format = modelData.value
                                }
                            }
                        }
                        Label {
                            Layout.fillWidth: true
                            text: win.formatHint
                            opacity: 0.65
                            font.pixelSize: 13
                            wrapMode: Text.Wrap
                        }

                        // ---- Calidad del MP3 ----
                        ColumnLayout {
                            Layout.fillWidth: true
                            visible: win.format === 5
                            spacing: 8
                            Label {
                                text: qsTr("CALIDAD DEL MP3")
                                font.pixelSize: 12
                                font.weight: Font.Medium
                                font.letterSpacing: 1
                                opacity: 0.55
                            }
                            Flow {
                                Layout.fillWidth: true
                                spacing: 8
                                Repeater {
                                    model: [128, 192, 256, 320]
                                    delegate: Chip {
                                        text: modelData + " kbps"
                                        selected: win.mp3kbps === modelData
                                        enabled: win.idle
                                        onClicked: win.mp3kbps = modelData
                                    }
                                }
                            }
                        }

                        // ---- Resolución del vídeo (solo YouTube + MP4) ----
                        ColumnLayout {
                            Layout.fillWidth: true
                            visible: win.videoFormat && !backend.spotify
                            spacing: 8
                            Label {
                                text: qsTr("RESOLUCIÓN")
                                font.pixelSize: 12
                                font.weight: Font.Medium
                                font.letterSpacing: 1
                                opacity: 0.55
                            }
                            ComboBox {
                                id: qualityBox
                                Layout.preferredWidth: 220
                                enabled: win.idle
                                model: backend.heights.map(h => h + "p" + (h > 1080 ? "  ·  AV1" : ""))
                                // El ComboBox reinicia su índice al cambiar el modelo, y lo hace después
                                // de esta señal: se aplaza un turno del bucle de eventos para ganarle.
                                onCountChanged: if (count > 0) Qt.callLater(() => currentIndex = win.defaultQualityIndex())
                            }
                            Label {
                                Layout.fillWidth: true
                                visible: win.chosenHeight > 1080
                                text: qsTr("Por encima de 1080p YouTube solo ofrece AV1; algunos reproductores antiguos no lo abren.")
                                wrapMode: Text.Wrap
                                font.pixelSize: 12
                                color: "#ffcc80"
                            }
                        }

                        Label {
                            Layout.fillWidth: true
                            visible: backend.spotify
                            text: qsTr("De Spotify solo se leen los datos (título, artista, portada). El audio se busca y se descarga de YouTube, y se guarda con sus etiquetas.")
                            wrapMode: Text.Wrap
                            font.pixelSize: 12
                            opacity: 0.6
                        }
                    }
                }

                // ---- Canciones (Spotify: álbum o playlist) ----
                Pane {
                    Layout.fillWidth: true
                    // Un Pane calcula su alto con el tamaño implícito de sus hijos, y un ListView
                    // no tiene: se le da el alto explícito (32 px por fila, máximo 220) y la lista lo rellena
                    Layout.preferredHeight: Math.min(backend.tracks.length * 32, 220) + 2 * padding
                    visible: backend.spotify && backend.tracks.length > 1
                    Material.elevation: 1
                    padding: 8

                    ListView {
                        id: trackList
                        anchors.fill: parent
                        clip: true
                        model: backend.tracks
                        boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: ScrollBar {}
                        delegate: Label {
                            width: trackList.width - 12
                            height: 32
                            leftPadding: 8
                            verticalAlignment: Text.AlignVCenter
                            text: (index + 1) + ".  " + modelData
                            elide: Text.ElideRight
                            opacity: 0.85
                        }
                    }
                }

                // ---- Carpeta ----
                RowLayout {
                    Layout.fillWidth: true
                    visible: backend.hasVideo
                    spacing: 12

                    Label {
                        text: qsTr("Guardar en")
                        opacity: 0.6
                    }
                    Label {
                        Layout.fillWidth: true
                        text: decodeURIComponent(win.folder.toString().replace("file://", ""))
                        elide: Text.ElideLeft
                    }
                    Button {
                        text: qsTr("Cambiar…")
                        flat: true
                        enabled: win.idle
                        onClicked: folderDialog.open()
                    }
                }

                // ---- Descargar / cancelar ----
                Button {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 52
                    visible: backend.hasVideo
                    enabled: backend.downloading || win.idle
                    highlighted: !backend.downloading
                    text: backend.downloading ? qsTr("Cancelar") : qsTr("Descargar")
                    font.pixelSize: 16
                    onClicked: backend.downloading
                               ? backend.cancel()
                               : backend.download(win.chosenHeight, win.format, win.mp3kbps, win.folder)
                }

                // ---- Progreso ----
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: backend.downloading || backend.status !== ""
                    spacing: 8

                    ProgressBar {
                        Layout.fillWidth: true
                        visible: backend.downloading
                        indeterminate: backend.progress < 0
                        value: Math.max(backend.progress, 0)
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Label {
                            text: backend.status
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }
                        Label {
                            visible: backend.downloading && backend.progress >= 0
                            text: backend.progressText + "  ·  " + Math.round(backend.progress * 100) + " %"
                            opacity: 0.7
                        }
                    }
                }

                // ---- Resultado ----
                Pane {
                    Layout.fillWidth: true
                    visible: backend.resultPath !== ""
                    Material.background: "#1f3b2a"
                    Material.elevation: 0
                    padding: 14

                    RowLayout {
                        width: parent.width
                        spacing: 12

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2
                            Label {
                                text: qsTr("Guardado")
                                font.weight: Font.Medium
                                color: "#a5d6a7"
                            }
                            Label {
                                Layout.fillWidth: true
                                text: backend.resultText !== "" ? backend.resultText : win.fileName(backend.resultPath)
                                elide: Text.ElideMiddle
                                opacity: 0.85
                            }
                        }
                        Button {
                            text: qsTr("Abrir carpeta")
                            onClicked: backend.showInFolder(backend.resultPath)
                        }
                    }
                }

                Item { Layout.preferredHeight: 4 }
            }
        }
    }
}
