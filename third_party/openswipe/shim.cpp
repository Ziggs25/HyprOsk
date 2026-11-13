// SPDX-FileCopyrightText: 2026 HyprOsk contributors
//
// SPDX-License-Identifier: MIT

#include "swipekb/swipe_engine.hpp"

#include <cstdint>
#include <cstring>
#include <new>
#include <string>
#include <utility>
#include <vector>

extern "C" {

struct HpSwipeEngine {
    swipekb::SwipeEngine engine;
};

HpSwipeEngine *hyprosk_swipe_engine_new(void)
{
    return new (std::nothrow) HpSwipeEngine();
}

void hyprosk_swipe_engine_free(HpSwipeEngine *handle)
{
    delete handle;
}

int hyprosk_swipe_engine_set_layout(HpSwipeEngine *handle, const uint32_t *labels,
                                    const float *geometry, size_t count)
{
    if (!handle || !labels || !geometry || count == 0 || count > 255)
        return 0;
    std::vector<swipekb::SwipeKey> keys;
    keys.reserve(count);
    for (size_t i = 0; i < count; ++i) {
        char32_t label = char32_t(labels[i]);
        if (label >= U'A' && label <= U'Z')
            label += 32;
        keys.push_back({label, geometry[i * 4], geometry[i * 4 + 1],
                        geometry[i * 4 + 2], geometry[i * 4 + 3]});
    }
    handle->engine.setLayout(swipekb::SwipeLayout(std::move(keys)));
    return 1;
}

int hyprosk_swipe_engine_refresh_geometry(HpSwipeEngine *handle)
{
    if (!handle)
        return 0;
    handle->engine.rebuildGeometry();
    return 1;
}

size_t hyprosk_swipe_engine_set_words(HpSwipeEngine *handle, const char *const *words,
                                      const uint8_t *freqs, size_t count)
{
    if (!handle || !words || !freqs)
        return 0;
    std::vector<std::pair<std::string, int>> list;
    list.reserve(count);
    for (size_t i = 0; i < count; ++i) {
        if (words[i] && words[i][0])
            list.emplace_back(words[i], int(freqs[i]));
    }
    return handle->engine.setWords(list);
}

int hyprosk_swipe_engine_ready(const HpSwipeEngine *handle)
{
    return handle && handle->engine.ready() ? 1 : 0;
}

int hyprosk_swipe_engine_decode(HpSwipeEngine *handle, const float *points,
                                const double *times, size_t count, float aspect,
                                int top_n, char *out, size_t out_cap, size_t *out_len)
{
    if (out_len)
        *out_len = 0;
    if (!handle || !points || !times || count == 0 || top_n <= 0 || !out || out_cap == 0)
        return -1;
    if (!handle->engine.ready())
        return -2;

    std::vector<swipekb::SwipePoint> trace;
    trace.reserve(count);
    for (size_t i = 0; i < count; ++i)
        trace.push_back({points[i * 2], points[i * 2 + 1], times[i]});

    const auto found = handle->engine.decode(trace, aspect, top_n);
    std::string joined;
    for (size_t i = 0; i < found.size(); ++i) {
        if (i)
            joined.push_back('\n');
        joined += handle->engine.lexicon().text(found[i].word);
    }
    if (joined.size() + 1 > out_cap)
        return -3;
    std::memcpy(out, joined.data(), joined.size());
    out[joined.size()] = '\0';
    if (out_len)
        *out_len = joined.size();
    return int(found.size());
}

}
