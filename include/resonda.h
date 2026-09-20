/* API C del núcleo de resonda (implementada en Rust, src/ffi.rs).
 * Enlazar con target/release/libresonda.a (+ pthread, dl, m). */
#ifndef RESONDA_H
#define RESONDA_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Códigos de retorno de rs_download */
#define RS_OK        0
#define RS_ERROR     1
#define RS_CANCELLED 2

/* Formatos de salida de rs_download */
#define RS_FORMAT_MP4 0 /* vídeo + audio */
#define RS_FORMAT_M4A 1 /* solo audio, AAC en MP4 */
#define RS_FORMAT_AAC 2 /* solo audio, AAC crudo (ADTS) */
#define RS_FORMAT_WAV 3 /* solo audio, PCM sin comprimir */
#define RS_FORMAT_FLAC 4 /* solo audio, sin pérdida */
#define RS_FORMAT_MP3 5 /* solo audio; calidad en RsOptions.bitrate_kbps */

/* Etapas que se notifican en el callback de progreso */
#define RS_STAGE_VIDEO  0
#define RS_STAGE_AUDIO  1
#define RS_STAGE_MUXING 2

typedef struct RsInfo RsInfo;     /* vídeo consultado (opaco) */
typedef struct RsCancel RsCancel; /* token de cancelación (opaco) */

typedef struct {
    uint32_t max_height;    /* altura máxima en píxeles; 0 = la mejor disponible */
    int format;             /* RS_FORMAT_* */
    uint32_t bitrate_kbps;  /* solo MP3: 128, 192, 256 o 320; 0 = 192 */
    const char *output_dir; /* carpeta de salida (UTF-8); NULL = carpeta actual */
} RsOptions;

/* `done`/`total` son bytes del conjunto de la descarga. Se puede llamar desde varios hilos. */
typedef void (*RsProgressCb)(int stage, uint64_t done, uint64_t total, void *user);

/* Toda cadena `char*` devuelta en un parámetro de salida se libera con rs_string_free. */
void rs_string_free(char *s);

/* Consulta un vídeo (URL o ID). Devuelve NULL si falla y deja el motivo en *err. */
RsInfo *rs_info_fetch(const char *input, char **err);
void rs_info_free(RsInfo *info);

/* Los punteros devueltos son propiedad de `info` (válidos hasta rs_info_free). */
const char *rs_info_title(const RsInfo *info);
const char *rs_info_id(const RsInfo *info);
uint64_t rs_info_seconds(const RsInfo *info);

/* Rellena `out` (capacidad `cap`) con las alturas disponibles, de mayor a menor.
 * Devuelve cuántas hay en total (puede superar `cap`). */
size_t rs_info_heights(const RsInfo *info, uint32_t *out, size_t cap);

RsCancel *rs_cancel_new(void);
void rs_cancel_set(const RsCancel *c);   /* pide cancelar; seguro desde cualquier hilo */
void rs_cancel_reset(const RsCancel *c); /* lo deja listo para otra descarga */
void rs_cancel_free(RsCancel *c);

/* Descarga el vídeo. BLOQUEA hasta terminar: llamar desde un hilo de trabajo.
 * Devuelve RS_OK (ruta del archivo en *out_path), RS_CANCELLED o RS_ERROR (motivo en *err).
 * `progress` y `cancel` pueden ser NULL. */
int rs_download(const RsInfo *info, const RsOptions *opts, RsProgressCb progress, void *user,
                const RsCancel *cancel, char **out_path, char **err);

/* ---- Spotify: se leen los DATOS del enlace (título, artista, portada); el audio se busca y se
 * descarga desde YouTube como M4A con etiquetas y portada. ---- */

typedef struct RsSpotify RsSpotify;

/* 1 si `input` es un enlace o URI de Spotify (canción, álbum o playlist). */
int rs_is_spotify_link(const char *input);

RsSpotify *rs_spotify_fetch(const char *input, char **err);
void rs_spotify_free(RsSpotify *s);

int rs_spotify_kind(const RsSpotify *s); /* 0 = canción, 1 = álbum, 2 = playlist */
/* Los punteros devueltos son propiedad de `s`. */
const char *rs_spotify_name(const RsSpotify *s);
const char *rs_spotify_cover(const RsSpotify *s); /* URL; puede ser "" */
size_t rs_spotify_count(const RsSpotify *s);
const char *rs_spotify_track_label(const RsSpotify *s, size_t i); /* "Artista - Título" */
uint64_t rs_spotify_track_seconds(const RsSpotify *s, size_t i);

/* Descarga la canción `i` en un formato de audio (RS_FORMAT_MP4 se trata como M4A). BLOQUEA: llamar desde un hilo de trabajo. Devuelve RS_OK (archivo en
 * *out_path y vídeo de YouTube usado en *matched), RS_CANCELLED o RS_ERROR (motivo en *err). */
int rs_spotify_download(const RsSpotify *s, size_t i, const char *output_dir, int format,
                        uint32_t bitrate_kbps, RsProgressCb progress, void *user, const RsCancel *cancel, char **out_path, char **matched,
                        char **err);

#ifdef __cplusplus
}
#endif

#endif /* RESONDA_H */
