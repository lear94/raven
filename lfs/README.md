# Linux From Scratch 13.1-systemd con Raven

Construye un sistema **LFS 13.1 (systemd)**, la versión estable más reciente
(1 de septiembre de 2026), usando Raven como gestor de paquetes: cada paquete del
libro es una receta, se compila en el sandbox de Raven y se instala de forma
transaccional. El sistema final queda gestionado por Raven (`raven list`).

## Estructura

| Ruta | Contenido |
| --- | --- |
| `recipes/cross/` | 23 recetas: capítulo 4.2 (estructura de `$LFS`), 5 (toolchain cruzado) y 6 (herramientas temporales). Se construyen en el host. |
| `recipes/temp/` | 8 recetas: herramientas temporales del capítulo 7 (dentro del chroot, base de datos aparte). |
| `recipes/system/` | 84 recetas: capítulos 7.5–7.6, 8 (80 paquetes), 9, 10 (kernel) y 11. `lfs-release` es la última y arrastra todas. |
| `recipes/CONVENTIONS.md` | Cómo se traducen los comandos del libro al modelo de Raven (`DESTDIR`, sandbox, post-install…). |
| `scripts/` | `01-cross.sh`, `02-chroot-enter.sh`, `03-in-chroot.sh`, `04-image.sh` y `build-all.sh` (todo seguido). |
| `fetch-sources.sh` | Descarga las fuentes de `sources/wget-list` y las verifica con los md5 del libro. |
| `tools/fill-checksums.py` | Rellena los `sha256_sum` de las recetas a partir de las fuentes verificadas. |
| `tools/gen-cross.py` | Generador de las recetas de `cross/`. |
| `Dockerfile` | Host de construcción (Debian trixie con los requisitos del §2.2 y Raven). |

## Requisitos

- Linux x86_64 o aarch64 con Docker (contenedor `--privileged`: Raven usa
  namespaces, chroot y montajes). En macOS sirve Colima:
  `brew install colima docker && colima start --vm-type vz --cpu 8 --memory 12 --disk 100`.
- ~40 GB de disco y varias horas de CPU.

## Uso

```sh
# Desde la raíz del repositorio
lfs/fetch-sources.sh                                 # opcional: descarga previa (~650 MB)
docker build -f lfs/Dockerfile -t raven-lfs .
docker run --rm --privileged \
  -v lfs-work:/work \
  -v "$PWD/lfs/sources":/lfs/sources \
  -v "$PWD/lfs/output":/output \
  raven-lfs                                          # ejecuta scripts/build-all.sh
```

El volumen `lfs-work` guarda el árbol LFS (`/work/lfs`) y los datos de Raven del
host (`/work/raven-cross`). Si algo falla, corrige la receta y vuelve a lanzar
el mismo comando: Raven no reconstruye lo que ya está instalado. Cada fase se
puede ejecutar por separado (`/lfs/scripts/01-cross.sh`, etc.).

Resultado en `lfs/output/`:

- `lfs-13.1.img`: sistema de ficheros ext4 (etiqueta `lfsroot`, sin tabla de particiones).
- `vmlinuz-7.1.8-lfs-13.1-systemd`: el kernel.

Arranque en QEMU (aarch64; en x86_64 usa `qemu-system-x86_64 -enable-kvm` y `console=ttyS0`):

```sh
qemu-system-aarch64 -M virt -cpu host -accel hvf -m 2G -nographic \
  -kernel lfs/output/vmlinuz-7.1.8-lfs-13.1-systemd \
  -append "root=/dev/vda rw console=ttyAMA0" \
  -drive file=lfs/output/lfs-13.1.img,if=virtio,format=raw \
  -netdev user,id=n0 -device virtio-net-pci,netdev=n0
```

## Cómo funciona

1. **Fase cruzada (host).** Raven instala en `$LFS` con `RAVEN_HOME=/work/raven-cross`.
   `RAVEN_SANDBOX_BINDS=$LFS` hace visible el toolchain cruzado dentro del sandbox
   (solo lectura) y `RAVEN_PASSENV="LFS LFS_TGT MAKEFLAGS"` pasa las variables del
   libro. Los paquetes se instalan con `DESTDIR=$DESTDIR$LFS`.
2. **Chroot.** `02-chroot-enter.sh` monta los sistemas de ficheros virtuales,
   copia el binario de Raven a `$LFS/usr/bin/raven` y entra en el chroot.
3. **Dentro del chroot.** `lfs-filesystem` (base de datos final), las herramientas
   temporales (base de datos `/var/lib/raven-lfs-temp`, para que el capítulo 8 pueda
   reemplazarlas), limpieza del §7.13 y `raven install lfs-release`, que construye
   los 80 paquetes del capítulo 8, la configuración del capítulo 9, el kernel y los
   ficheros de versión. Al final se hace la limpieza del §8.86.
4. **Imagen.** `04-image.sh` genera la imagen ext4 con `mke2fs -d`.

## Diferencias con el libro

- **Sin test suites** (`make check`): ahorran horas; el libro considera críticas las
  de glibc, gcc, binutils, gmp y mpfr. Hay un comentario en cada una de esas recetas.
- **Configuración no interactiva:** zona horaria UTC, `LANG=C.UTF-8`, hostname `lfs`,
  DHCP en todas las interfaces cableadas (`en*`, `eth*`), `/etc/fstab` con
  `LABEL=lfsroot`, sin contraseña de root (`passwd root` antes de arrancar).
- **Kernel:** `defconfig` + `kvm_guest.config` + las opciones que exige el §10.3.
  Para usar tu propia configuración, colócala en `/etc/raven/linux.config` dentro del
  chroot antes de construir `linux`.
- **GRUB** se compila (§8) pero no se instala en ningún disco: la imagen arranca con
  el kernel directamente. Para un disco real sigue el §10.4 (`grub-install`, `grub.cfg`).
- **aarch64:** el libro solo cubre x86_64. Las recetas de GCC añaden el equivalente
  de su `sed` de `lib64` para `t-aarch64-linux`.
- No se aplica el *stripping* opcional del §8.85.
- `/usr/share/info/dir` no pertenece a ningún paquete (`RAVEN_NO_EXTRACT`); se
  regenera al final.
