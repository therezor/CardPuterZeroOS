# gh0st-mediad, the media app's decoder:
#   make O=OUT                  the helper
#   make check O=OUT CORPUS=DIR  host checks: every file corpus.sh wrote,
#                                then the shared JPEG reader's own
#   make playcheck O=OUT        the player against the silent stub server
# Everything is written under O, so the sources may be read-only. The
# picture, player, link and ring sources are shared with gh0st-camd, in
# native/av.

O ?= build
NATIVE ?= $(abspath ../../../native/av)
VPATH = $(NATIVE)
CXX ?= c++
CXXFLAGS ?= -O2
CXXFLAGS += -std=c++17 -Wall -Wextra -MMD -MP -I. -I$(NATIVE)
LIBS := libjpeg libavformat libavcodec libavfilter libswscale libswresample libavutil \
	libheif libraw libwebpdemux libwebp libass
DEP_CFLAGS ?= $(shell pkg-config --cflags $(LIBS) 2>/dev/null)
DEP_LIBS ?= $(shell pkg-config --libs $(LIBS) 2>/dev/null)
PULSE_CFLAGS ?= $(shell pkg-config --cflags libpulse-simple 2>/dev/null)
PULSE_LIBS ?= $(shell pkg-config --libs libpulse-simple libpulse 2>/dev/null)

common := main link ring decode stills cache player

all: $(O)/gh0st-mediad

$(O)/gh0st-mediad: $(addprefix $(O)/,$(addsuffix .o,$(common) pulse))
	$(CXX) -s -o $@ $^ $(DEP_LIBS) $(PULSE_LIBS) -pthread -Wl,--as-needed

check: $(O)/media_check $(O)/decode_check
	$(O)/decode_check
	$(O)/media_check $(CORPUS)

$(O)/media_check: $(addprefix $(O)/,$(addsuffix .o,media_check stills decode cache))
	$(CXX) -o $@ $^ $(DEP_LIBS) -pthread

$(O)/decode_check: $(O)/decode_check.o
	$(CXX) -o $@ $^ $(DEP_LIBS) -pthread

playcheck: $(O)/playcheck

$(O)/playcheck: $(addprefix $(O)/,$(addsuffix .o,playcheck player pulse_stub decode))
	$(CXX) -o $@ $^ $(DEP_LIBS) -pthread

$(O)/%.o: %.cpp | $(O)
	$(CXX) $(CXXFLAGS) $(DEP_CFLAGS) $(PULSE_CFLAGS) -c -o $@ $<

$(O):
	mkdir -p $@

clean:
	rm -rf $(O)

.PHONY: all check playcheck clean
-include $(wildcard $(O)/*.d)
