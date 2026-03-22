/*
 * FairPlay sender-side encryption for AirPlay screen mirroring.
 *
 * Uses the playfair crypto primitives (from UxPlay, GPL-licensed) to
 * implement the sender side of the FairPlay key exchange:
 *   1. Construct a 164-byte fp-setup step 2 message
 *   2. Derive the same session key the receiver will derive
 *   3. Encrypt our AES key into a 72-byte ekey the receiver can decrypt
 */

#include <string.h>
#include <stdint.h>
#include "playfair/playfair.h"

/* Functions from omg_hax.c */
extern void generate_session_key(unsigned char* oldSap, unsigned char* messageIn, unsigned char* sessionKey);
extern void generate_key_schedule(unsigned char* key_material, uint32_t key_schedule[11][4]);
extern void cycle(unsigned char* block, uint32_t key_schedule[11][4]);
extern void z_xor(unsigned char* in, unsigned char* out, int blocks);
extern void x_xor(unsigned char* in, unsigned char* out, int blocks);
extern unsigned char default_sap[];

/*
 * Encrypt a 16-byte AES key into a 72-byte FairPlay ekey.
 *
 * message3:     the 164-byte fp-setup step 2 message (same one sent to the receiver)
 * aes_key:      the 16-byte AES key we want the receiver to end up with
 * ekey_out:     output buffer, must be at least 72 bytes
 *
 * The receiver will call playfair_decrypt(message3, ekey_out, recovered_key)
 * and get back our aes_key.
 */
void fairplay_encrypt(const unsigned char* message3,
                      const unsigned char* aes_key,
                      unsigned char* ekey_out)
{
    unsigned char sap_key[16];
    uint32_t key_schedule[11][4];

    /* 1. Derive session key (same as receiver will do) */
    generate_session_key(default_sap, (unsigned char*)message3, sap_key);
    generate_key_schedule(sap_key, key_schedule);

    /* 2. Choose chunk2 (bytes [56:72] of ekey) — can be anything */
    unsigned char chunk2[16];
    for (int i = 0; i < 16; i++)
        chunk2[i] = aes_key[i] ^ 0x5A ^ (unsigned char)i;

    /* 3. Forward-compute cycle output from chunk2 */
    unsigned char block[16];
    z_xor(chunk2, block, 1);       /* block = chunk2 ^ z_key */
    cycle(block, key_schedule);     /* block = cycle(block)   */

    /* 4. Compute intermediate = aes_key with final XORs undone
     *    decrypt does: keyOut ^= x_key; keyOut ^= z_key
     *    so reverse:   intermediate = aes_key ^ z_key ^ x_key */
    unsigned char intermediate[16];
    memcpy(intermediate, aes_key, 16);
    z_xor(intermediate, intermediate, 1);  /* ^= z_key */
    x_xor(intermediate, intermediate, 1);  /* ^= x_key */

    /* 5. chunk1 = cycle_output ^ intermediate
     *    decrypt does: keyOut = cycle_output ^ chunk1
     *    so: chunk1 = cycle_output ^ intermediate */
    unsigned char chunk1[16];
    for (int i = 0; i < 16; i++)
        chunk1[i] = block[i] ^ intermediate[i];

    /* 6. Assemble 72-byte ekey:
     *    [0:16]  = padding (unused by decrypt)
     *    [16:32] = chunk1
     *    [32:56] = padding (unused by decrypt)
     *    [56:72] = chunk2 */
    memset(ekey_out, 0, 72);
    memcpy(ekey_out + 16, chunk1, 16);
    memcpy(ekey_out + 56, chunk2, 16);
}
