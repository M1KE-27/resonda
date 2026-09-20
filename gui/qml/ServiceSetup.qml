import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Effects
import QtQuick.Layouts

// Pantalla de bienvenida: se eligen los servicios que se quieren usar. Solo YouTube y Spotify
// funcionan hoy; el resto aparece como "Próximamente".
Item {
    id: root

    // Servicios marcados al abrir la pantalla
    property var initial: ["youtube", "spotify"]
    property bool firstRun: true
    signal accepted(var ids)
    signal cancelled()

    property var selected: initial.slice()

    // Siempre se abre desde arriba (el contenido cambia de alto mientras se calcula el diseño y
    // el desplazamiento se quedaba a mitad de lista)
    function scrollToTop() {
        scroll.contentItem.contentY = 0
    }
    onVisibleChanged: if (visible) Qt.callLater(scrollToTop)
    Component.onCompleted: Qt.callLater(scrollToTop)

    readonly property var live: [
        { id: "youtube", name: "YouTube", logo: "youtube.svg", desc: qsTr("Vídeos y audio, de cualquier duración") },
        { id: "spotify", name: "Spotify", logo: "spotify.svg", desc: qsTr("Canciones, álbumes y playlists") }
    ]
    readonly property var soon: [
        { name: "Apple Music", logo: "apple-music.svg" }, { name: "SoundCloud", logo: "soundcloud.svg" },
        { name: "Bandcamp", logo: "bandcamp.svg" }, { name: "Deezer", logo: "deezer.svg" },
        { name: "Tidal", logo: "tidal.svg" },
        // Icono a todo color (PNG) aportado aparte: el SVG de la carpeta era una reconstrucción manual
        { name: "Amazon Music", logo: "amazon-music.png" },
        { name: "Vimeo", logo: "vimeo.svg" }, { name: "Twitch", logo: "twitch.svg" },
        { name: "TikTok", logo: "tiktok.svg" }, { name: "Instagram", logo: "instagram.svg" },
        { name: "Dailymotion", logo: "dailymotion.svg" }, { name: "Facebook", logo: "facebook.svg" },
        { name: "Reddit", logo: "reddit.svg" }, { name: "X (Twitter)", logo: "x.svg" }
    ]

    function isOn(id) { return selected.indexOf(id) >= 0 }
    function toggle(id) {
        const next = selected.filter(x => x !== id)
        if (next.length === selected.length)
            next.push(id)
        selected = next
    }

    component ServiceCard: Rectangle {
        id: card
        property string name: ""
        property string desc: ""
        property string logo: ""                       // nombre de archivo en logos/ (.svg o .png)
        readonly property bool fullColor: logo.endsWith(".png") // PNG = icono a todo color, se muestra tal cual
        property bool available: true
        property bool on: false
        signal toggled()

        Layout.fillWidth: true
        Layout.preferredHeight: 76
        radius: 14
        color: available && on ? "#2b2b33" : "#222227"
        border.width: available && on ? 2 : 1
        border.color: available && on ? Material.accent : "#33ffffff"
        opacity: available ? 1 : 0.7

        MouseArea {
            anchors.fill: parent
            enabled: card.available
            cursorShape: Qt.PointingHandCursor
            onClicked: card.toggled()
        }

        RowLayout {
            anchors { fill: parent; leftMargin: 16; rightMargin: 16 }
            spacing: 14

            Rectangle {
                id: tile
                Layout.preferredWidth: 44
                Layout.preferredHeight: 44
                radius: 12
                // Ficha neutra: el color lo trae el propio logo (SVG con su color de marca, o PNG a todo
                // color con su fondo, que se muestra recortado con las esquinas redondeadas)
                color: card.fullColor && logoImage.status === Image.Ready ? "transparent" : "#34343c"

                Image {
                    id: logoImage
                    anchors.centerIn: parent
                    width: card.fullColor ? parent.width : 26
                    height: width
                    sourceSize: Qt.size(96, 96)
                    smooth: true
                    fillMode: Image.PreserveAspectFit
                    source: card.logo !== "" ? "qrc:/qt/qml/Resonda/logos/" + card.logo : ""
                    // Los SVG se ven tal cual; el PNG pasa por el efecto de máscara (abajo)
                    visible: !card.fullColor && status === Image.Ready
                }
                // Máscara con las esquinas redondeadas de la ficha (para los iconos a todo color)
                Rectangle {
                    id: roundMask
                    anchors.fill: parent
                    radius: tile.radius
                    visible: false
                    layer.enabled: true
                }
                MultiEffect {
                    anchors.fill: logoImage
                    source: logoImage
                    maskEnabled: true
                    maskSource: roundMask
                    visible: card.fullColor && logoImage.status === Image.Ready
                }
                // Si no hay logo (o no se pudo cargar): la inicial
                Label {
                    anchors.centerIn: parent
                    visible: logoImage.status !== Image.Ready
                    text: card.name.charAt(0)
                    font.pixelSize: 22
                    font.weight: Font.Bold
                    color: "white"
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 1
                Label {
                    text: card.name
                    font.pixelSize: 16
                    font.weight: Font.Medium
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Label {
                    visible: card.desc !== ""
                    text: card.desc
                    font.pixelSize: 12
                    opacity: 0.65
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
            }
            Switch {
                visible: card.available
                checked: card.on
                onToggled: card.toggled()
            }
            Rectangle {
                visible: !card.available
                radius: 10
                color: "#33ffffff"
                Layout.preferredWidth: soonLabel.implicitWidth + 20
                Layout.preferredHeight: 24
                Label {
                    id: soonLabel
                    anchors.centerIn: parent
                    text: qsTr("Próximamente")
                    font.pixelSize: 11
                    font.weight: Font.Medium
                }
            }
        }
    }

    ScrollView {
        id: scroll
        anchors.fill: parent
        contentWidth: availableWidth
        clip: true

        ColumnLayout {
            width: scroll.availableWidth
            spacing: 0

            ColumnLayout {
                Layout.fillWidth: true
                Layout.margins: 28
                spacing: 20

                ColumnLayout {
                    spacing: 4
                    Label {
                        text: root.firstRun ? qsTr("Bienvenido") : qsTr("Servicios")
                        font.pixelSize: 30
                        font.weight: Font.Bold
                    }
                    Label {
                        Layout.fillWidth: true
                        text: qsTr("Elige los servicios que quieres usar. Podrás cambiarlo cuando quieras desde «Servicios», arriba a la derecha.")
                        wrapMode: Text.Wrap
                        opacity: 0.7
                    }
                }

                Label {
                    text: qsTr("DISPONIBLES")
                    font.pixelSize: 12
                    font.weight: Font.Medium
                    font.letterSpacing: 1
                    opacity: 0.55
                }
                GridLayout {
                    Layout.fillWidth: true
                    columns: scroll.availableWidth > 640 ? 2 : 1
                    columnSpacing: 12
                    rowSpacing: 12
                    Repeater {
                        model: root.live
                        delegate: ServiceCard {
                            name: modelData.name
                            logo: modelData.logo
                            desc: modelData.desc
                                            on: root.isOn(modelData.id)
                            onToggled: root.toggle(modelData.id)
                        }
                    }
                }

                Label {
                    Layout.topMargin: 8
                    text: qsTr("PRÓXIMAMENTE")
                    font.pixelSize: 12
                    font.weight: Font.Medium
                    font.letterSpacing: 1
                    opacity: 0.55
                }
                GridLayout {
                    Layout.fillWidth: true
                    columns: scroll.availableWidth > 640 ? 2 : 1
                    columnSpacing: 12
                    rowSpacing: 12
                    Repeater {
                        model: root.soon
                        delegate: ServiceCard {
                            name: modelData.name
                            logo: modelData.logo
                            available: false
                        }
                    }
                }

                RowLayout {
                    Layout.topMargin: 8
                    Layout.fillWidth: true
                    spacing: 12
                    Button {
                        visible: !root.firstRun
                        flat: true
                        text: qsTr("Cancelar")
                        onClicked: root.cancelled()
                    }
                    Item { Layout.fillWidth: true }
                    Label {
                        visible: root.selected.length === 0
                        text: qsTr("Elige al menos un servicio")
                        color: "#ffb4ab"
                        font.pixelSize: 12
                    }
                    Button {
                        highlighted: true
                        enabled: root.selected.length > 0
                        text: root.firstRun ? qsTr("Continuar") : qsTr("Guardar")
                        Layout.preferredHeight: 46
                        Layout.preferredWidth: 160
                        onClicked: root.accepted(root.selected)
                    }
                }

                Label {
                    Layout.fillWidth: true
                    text: qsTr("Los logotipos (de Simple Icons) son marcas registradas de sus propietarios. Resonda es independiente y no está afiliada a ninguno de estos servicios.")
                    wrapMode: Text.Wrap
                    font.pixelSize: 11
                    opacity: 0.45
                }

                Item { Layout.preferredHeight: 4 }
            }
        }
    }
}
