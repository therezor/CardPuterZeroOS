# gh0st-camd. Two builds:
#   make SYSROOT=DIR O=OUT   the real helper, against libcamera unpacked in
#                            DIR (os/image/build-image.sh does this)
#   make stub O=OUT          a test-pattern sensor for any host with
#                            ffmpeg's libraries, and no sound server
#   make check O=OUT         host checks of the file readers
# Everything is written under O, so the sources may be read-only. The
# picture, player, link and ring sources are shared with gh0st-mediad, in
# native/av.

O ?= build
NATIVE ?= $(abspath ../../../native/av)
VPATH = $(NATIVE)
CXX ?= c++
CXXFLAGS ?= -O2
CXXFLAGS += -std=c++17 -Wall -Wextra -MMD -MP -I. -I$(NATIVE)
JPEG_CFLAGS ?= $(shell pkg-config --cflags libjpeg 2>/dev/null)
JPEG_LIBS ?= $(shell pkg-config --libs libjpeg 2>/dev/null || echo -ljpeg)
# Sound and playback: AAC in, H.264 and AAC out. The image has these for
# ffmpeg already.
AV := libavformat libavcodec libswscale libswresample libavutil
AV_CFLAGS ?= $(shell pkg-config --cflags $(AV) 2>/dev/null)
AV_LIBS ?= $(shell pkg-config --libs $(AV) 2>/dev/null)
PULSE_CFLAGS ?= $(shell pkg-config --cflags libpulse-simple 2>/dev/null)
PULSE_LIBS ?= $(shell pkg-config --libs libpulse-simple libpulse 2>/dev/null)

common := main link ring still decode mp4 mic player

ifdef SYSROOT
LIBDIR := $(SYSROOT)/usr/lib/aarch64-linux-gnu
CAM_CFLAGS := -I$(SYSROOT)/usr/include/libcamera
# libcamera's own dependencies are on the device, not in the build
# container: resolve what we call, leave the rest to the device's loader.
CAM_LIBS := -L$(LIBDIR) -Wl,-rpath-link,$(LIBDIR) -lcamera -lcamera-base \
	-Wl,--allow-shlib-undefined
endif

all: $(O)/gh0st-camd

$(O)/gh0st-camd: $(addprefix $(O)/,$(addsuffix .o,$(common) cam_libcamera rec venc pulse))
	$(CXX) -s -o $@ $^ $(CAM_LIBS) $(JPEG_LIBS) $(AV_LIBS) $(PULSE_LIBS) -pthread -Wl,--as-needed

stub: $(O)/gh0st-camd-stub

$(O)/gh0st-camd-stub: $(addprefix $(O)/,$(addsuffix .o,$(common) cam_stub rec_stub pulse_stub))
	$(CXX) -o $@ $^ $(JPEG_LIBS) $(AV_LIBS) -pthread

# A host check of the MP4 writer: wraps a raw H.264 file.
mp4cat: $(O)/mp4cat

$(O)/mp4cat: $(O)/mp4cat.o $(O)/mp4.o
	$(CXX) -o $@ $^ -pthread

# A host check of the player against the silent stub server.
playcheck: $(O)/playcheck

$(O)/playcheck: $(addprefix $(O)/,$(addsuffix .o,playcheck player pulse_stub decode))
	$(CXX) -o $@ $^ $(AV_LIBS) $(JPEG_LIBS) -pthread

# Host checks of what reads files from elsewhere (decode_check.cpp).
check: $(O)/decode_check
	$(O)/decode_check

$(O)/decode_check: $(O)/decode_check.o
	$(CXX) -o $@ $^ $(JPEG_LIBS) -pthread

$(O)/%.o: %.cpp | $(O)
	$(CXX) $(CXXFLAGS) $(JPEG_CFLAGS) $(AV_CFLAGS) $(PULSE_CFLAGS) $(CAM_CFLAGS) -c -o $@ $<

$(O):
	mkdir -p $@

clean:
	rm -rf $(O)

.PHONY: all stub mp4cat playcheck check clean
-include $(wildcard $(O)/*.d)
