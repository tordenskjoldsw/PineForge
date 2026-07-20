/* PineTime MCUBoot application image.
 * The bootloader owns 0x0000..0x7fff. imgtool adds a 32-byte header at 0x8000,
 * therefore the Rust vector table starts at 0x8020.
 */
MEMORY
{
  FLASH : ORIGIN = 0x00008020, LENGTH = 0x00073FE0
  RAM   : ORIGIN = 0x20000008, LENGTH = 0x0000FFF8
}
