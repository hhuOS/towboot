/*  boot.S - bootstrap the kernel */
/*  Copyright (C) 1999, 2001, 2010  Free Software Foundation, Inc.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 */

#define ASM_FILE        1
#include "multiboot2.h"

/*  C symbol format. HAVE_ASM_USCORE is defined by configure. */
#ifdef HAVE_ASM_USCORE
# define EXT_C(sym)                     _ ## sym
#else
# define EXT_C(sym)                     sym
#endif

/*  The size of our stack (16KB). */
#define STACK_SIZE                      0x4000

/*  The flags for the Multiboot header. */
#ifdef __ELF__
# define AOUT_KLUDGE 0
#else
# define AOUT_KLUDGE MULTIBOOT_AOUT_KLUDGE
#endif
        
        .text

        .globl  start, _start
.code32
/* this just exists to crash gracefully on 32-bit systems */
start:
_start:
        hlt
        jmp     _start
.code64

        /*  Align 64 bits boundary. */
        .align  8
        
        /*  Multiboot header. */
multiboot_header:
        /*  magic */
        .long   MULTIBOOT2_HEADER_MAGIC
        /*  ISA: i386 */
        .long   MULTIBOOT_ARCHITECTURE_I386
        /*  Header length. */
        .long   multiboot_header_end - multiboot_header
        /*  checksum */
        .long   -(MULTIBOOT2_HEADER_MAGIC + MULTIBOOT_ARCHITECTURE_I386 + (multiboot_header_end - multiboot_header))
#ifndef __ELF__
        .align 8
address_tag_start:      
        .short MULTIBOOT_HEADER_TAG_ADDRESS
        .short MULTIBOOT_HEADER_TAG_OPTIONAL
        .long address_tag_end - address_tag_start
        /*  header_addr */
        .long   multiboot_header
        /*  load_addr */
        .long   _start
        /*  load_end_addr */
        .long   _edata
        /*  bss_end_addr */
        .long   _end
address_tag_end:
#endif /*  __ELF__ */
        .align 8
entry_address_tag_start:        
        .short MULTIBOOT_HEADER_TAG_ENTRY_ADDRESS_EFI64
        .short MULTIBOOT_HEADER_TAG_OPTIONAL
        .long entry_address_tag_end - entry_address_tag_start
        /*  entry_addr */
        .long multiboot_entry
entry_address_tag_end:
        .align 8
boot_services_tag_start:
        .short MULTIBOOT_HEADER_TAG_EFI_BS
        .short MULTIBOOT_HEADER_TAG_OPTIONAL
        .long boot_services_tag_end - boot_services_tag_start
boot_services_tag_end:
        .align 8
framebuffer_tag_start:  
        .short MULTIBOOT_HEADER_TAG_FRAMEBUFFER
        .short MULTIBOOT_HEADER_TAG_OPTIONAL
        .long framebuffer_tag_end - framebuffer_tag_start
        .long 1024
        .long 768
        .long 32
framebuffer_tag_end:
        .align 8
end_tag_start:
        .short MULTIBOOT_HEADER_TAG_END
        .short 0
        .long end_tag_end - end_tag_start
end_tag_end:
        .align 8
multiboot_header_end:
multiboot_entry:
        /*  Initialize the stack pointer. */
        movq    $(stack + STACK_SIZE), %rsp

        /*  Reset EFLAGS. */
        pushq   $0
        popf

        /* parameter 2: the pointer to the Multiboot information structure. */
        movq    $0, %rsi
        mov     %ebx, %esi
        /* parameter 1: the magic value. */
        movq    $0, %rdi
        mov     %eax, %edi

        /*  Now enter the C main function... */
        call    EXT_C(cmain)

        /*  Halt. */
        movq   $halt_message, %rdi
        call    EXT_C(puts)
        
loop:   hlt
        jmp     loop

halt_message:
        .asciz  "Halted."

        /*  Our stack area. */
        .comm   stack, STACK_SIZE
