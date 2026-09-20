cmake_minimum_required(VERSION 3.21)
project(resonda-gui LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 6.5 REQUIRED COMPONENTS Quick QuickControls2)
find_package(Threads REQUIRED)
qt_standard_project_setup(REQUIRES 6.5)
if(QT_KNOWN_POLICY_QTP0004)
    qt_policy(SET QTP0004 NEW)
endif()

# ---- Núcleo Rust (biblioteca estática con API C, ver ../include/resonda.h) ----
find_program(CARGO cargo REQUIRED)
set(RUST_ROOT ${CMAKE_CURRENT_SOURCE_DIR}/..)
if(MSVC)
    set(RUST_LIB ${RUST_ROOT}/target/release/resonda.lib)
else()
    set(RUST_LIB ${RUST_ROOT}/target/release/libresonda.a)
endif()
add_custom_target(rust_core ALL
    COMMAND ${CARGO} build --release --lib --manifest-path ${RUST_ROOT}/Cargo.toml
    BYPRODUCTS ${RUST_LIB}
    COMMENT "Compilando el núcleo Rust"
    USES_TERMINAL)
add_library(resonda_core STATIC IMPORTED)
set_target_properties(resonda_core PROPERTIES IMPORTED_LOCATION ${RUST_LIB})
add_dependencies(resonda_core rust_core)

# ---- Interfaz Qt/QML ----
# Logos de los servicios (assets/logos): se incrustan en el binario
file(GLOB LOGO_FILES CONFIGURE_DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/assets/logos/*.svg ${CMAKE_CURRENT_SOURCE_DIR}/assets/logos/*.png)
foreach(f ${LOGO_FILES})
    get_filename_component(logo_name ${f} NAME)
    set_source_files_properties(${f} PROPERTIES QT_RESOURCE_ALIAS logos/${logo_name})
endforeach()

qt_add_executable(resonda-gui src/main.cpp)
qt_add_qml_module(resonda-gui
    URI Resonda
    VERSION 1.0
    QML_FILES qml/Main.qml qml/ServiceSetup.qml
    RESOURCES ${LOGO_FILES}
    SOURCES src/backend.h src/backend.cpp)

target_include_directories(resonda-gui PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src ${RUST_ROOT}/include)
target_link_libraries(resonda-gui PRIVATE Qt6::Quick Qt6::QuickControls2 resonda_core Threads::Threads)
if(WIN32)
    # Bibliotecas del sistema que necesita el núcleo Rust (std, sockets, criptografía)
    target_link_libraries(resonda-gui PRIVATE advapi32 bcrypt ntdll userenv ws2_32 synchronization)
    set_target_properties(resonda-gui PROPERTIES WIN32_EXECUTABLE ON)
else()
    target_link_libraries(resonda-gui PRIVATE ${CMAKE_DL_LIBS} m rt util)
endif()

install(TARGETS resonda-gui RUNTIME DESTINATION bin)
install(FILES data/resonda.desktop DESTINATION share/applications)
install(FILES data/resonda.svg DESTINATION share/icons/hicolor/scalable/apps)
