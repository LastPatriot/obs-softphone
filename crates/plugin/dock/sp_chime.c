/*
 * SPDX-License-Identifier: GPL-2.0-or-later
 *
 * The ring chime (DESIGN.md §5.1): a soft two-tone chime played through the
 * operating system's own sound API, so it can never reach OBS's mix, the
 * stream or the caller. macOS: AudioServices (system sound output);
 * Windows: PlaySound. Elsewhere it is silent.
 */
#define _USE_MATH_DEFINES /* M_PI on MSVC */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef __APPLE__
#include <AudioToolbox/AudioToolbox.h>
#include <CoreFoundation/CoreFoundation.h>
#elif defined(_WIN32)
#include <windows.h>
#include <mmsystem.h>
#endif

#define RATE 22050
#define AMPLITUDE 0.2 /* about -14 dBFS: quiet */

static unsigned char *g_wav;
static size_t g_wav_len;

#ifdef __APPLE__
static SystemSoundID g_sound;
static int g_sound_ok;
#endif

static void put_u32(unsigned char *p, uint32_t v)
{
	p[0] = v & 0xff, p[1] = (v >> 8) & 0xff, p[2] = (v >> 16) & 0xff, p[3] = (v >> 24) & 0xff;
}

static void put_u16(unsigned char *p, uint16_t v)
{
	p[0] = v & 0xff, p[1] = (v >> 8) & 0xff;
}

/* Appends one tone with a 10 ms fade in and a fade out over its last half. */
static size_t tone(int16_t *out, double freq, double secs)
{
	size_t n = (size_t)(secs * RATE), fade_in = RATE / 100, i;

	for (i = 0; i < n; ++i) {
		double env = 1.0;
		if (i < fade_in)
			env = (double)i / fade_in;
		else if (i > n / 2)
			env = (double)(n - i) / (n - n / 2);
		out[i] = (int16_t)(32767.0 * AMPLITUDE * env * sin(2.0 * M_PI * freq * i / RATE));
	}
	return n;
}

static int build_wav(void)
{
	int16_t samples[RATE]; /* up to 1 s */
	size_t n = 0, data_len;

	n += tone(samples + n, 880.0, 0.14);
	n += tone(samples + n, 660.0, 0.22);
	data_len = n * sizeof(int16_t);

	g_wav_len = 44 + data_len;
	g_wav = malloc(g_wav_len);
	if (!g_wav)
		return -1;
	memcpy(g_wav, "RIFF", 4);
	put_u32(g_wav + 4, (uint32_t)(g_wav_len - 8));
	memcpy(g_wav + 8, "WAVEfmt ", 8);
	put_u32(g_wav + 16, 16);
	put_u16(g_wav + 20, 1); /* PCM */
	put_u16(g_wav + 22, 1); /* mono */
	put_u32(g_wav + 24, RATE);
	put_u32(g_wav + 28, RATE * 2);
	put_u16(g_wav + 32, 2);
	put_u16(g_wav + 34, 16);
	memcpy(g_wav + 36, "data", 4);
	put_u32(g_wav + 40, (uint32_t)data_len);
	memcpy(g_wav + 44, samples, data_len); /* little-endian hosts */
	return 0;
}

/* `path`: where to write chime.wav (macOS plays system sounds from a file). */
int sp_chime_init(const char *path)
{
	if (!g_wav && build_wav() != 0)
		return -1;
#ifdef __APPLE__
	{
		FILE *f = fopen(path, "wb");
		CFURLRef url;

		if (!f)
			return -1;
		fwrite(g_wav, 1, g_wav_len, f);
		fclose(f);
		url = CFURLCreateFromFileSystemRepresentation(NULL, (const UInt8 *)path, (CFIndex)strlen(path), false);
		if (!url)
			return -1;
		if (g_sound_ok)
			AudioServicesDisposeSystemSoundID(g_sound);
		g_sound_ok = AudioServicesCreateSystemSoundID(url, &g_sound) == kAudioServicesNoError;
		CFRelease(url);
		return g_sound_ok ? 0 : -1;
	}
#else
	(void)path;
	return 0;
#endif
}

void sp_chime_play(void)
{
#ifdef __APPLE__
	if (g_sound_ok)
		AudioServicesPlaySystemSound(g_sound);
#elif defined(_WIN32)
	if (g_wav)
		PlaySoundA((LPCSTR)g_wav, NULL, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
#endif
}
