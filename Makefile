# Resonda: atajos de compilación. `make help` muestra las opciones.
.PHONY: help cli gui run test clean

help:
	@echo "make cli    compila la terminal  -> target/release/resonda"
	@echo "make gui    compila la ventana   -> gui/build/resonda-gui"
	@echo "make run    compila y abre la ventana"
	@echo "make test   ejecuta los tests del núcleo"
	@echo "make clean  borra todo lo compilado (deja solo el código)"

cli:
	cargo build --release

gui:
	cmake -S gui -B gui/build -G Ninja -DCMAKE_BUILD_TYPE=Release
	cmake --build gui/build

run: gui
	./gui/build/resonda-gui

test:
	cargo test --release

clean:
	cargo clean
	rm -rf gui/build
