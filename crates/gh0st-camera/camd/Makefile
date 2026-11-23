# gh0st-camd. Two builds:
#   make SYSROOT=DIR O=OUT   the real helper, against libcamera unpacked in
#                            DIR (os/image/build-image.sh does this)
#   make stub O=OUT          a test-pattern sensor for any host
# Everything is written under O, so the sources may be read-only.

O ?= build
CXX ?= c++
CXXFLAGS ?= -O2
CXXFLAGS += -std=c++17 -Wall -Wextra -MMD -MP
JPEG_CFLAGS ?= $(shell pkg-config --cflags libjpeg 2>/dev/null)
JPEG_LIBS ?= $(shell pkg-config --libs libjpeg 2>/dev/null || echo -ljpeg)

common := main link ring still mp4

ifdef SYSROOT
LIBDIR := $(SYSROOT)/usr/lib/aarch64-linux-gnu
CAM_CFLAGS := -I$(SYSROOT)/usr/include/libcamera
# libcamera's own dependencies are on the device, not in the build
# container: resolve what we call, leave the rest to the device's loader.
CAM_LIBS := -L$(LIBDIR) -Wl,-rpath-link,$(LIBDIR) -lcamera -lcamera-base \
	-Wl,--allow-shlib-undefined
endif

all: $(O)/gh0st-camd

$(O)/gh0st-camd: $(addprefix $(O)/,$(addsuffix .o,$(common) cam_libcamera rec venc))
	$(CXX) -s -o $@ $^ $(CAM_LIBS) $(JPEG_LIBS) -pthread -Wl,--as-needed

stub: $(O)/gh0st-camd-stub

$(O)/gh0st-camd-stub: $(addprefix $(O)/,$(addsuffix .o,$(common) cam_stub rec_stub))
	$(CXX) -o $@ $^ $(JPEG_LIBS) -pthread

# A host check of the MP4 writer: wraps a raw H.264 file.
mp4cat: $(O)/mp4cat

$(O)/mp4cat: $(O)/mp4cat.o $(O)/mp4.o
	$(CXX) -o $@ $^ -pthread

$(O)/%.o: %.cpp | $(O)
	$(CXX) $(CXXFLAGS) $(JPEG_CFLAGS) $(CAM_CFLAGS) -c -o $@ $<

$(O):
	mkdir -p $@

clean:
	rm -rf $(O)

.PHONY: all stub mp4cat clean
-include $(wildcard $(O)/*.d)
