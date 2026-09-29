// Opt-in instrumentation only. Timing from an attached worker is not a baseline.
// Hooks aggregate in native code; no JavaScript or stack unwinding on heap calls.
const arena = Memory.alloc(8 * 1024 * 1024);
const cm = new CModule(`
#include <gum/guminterceptor.h>
#include <glib.h>
#include <stdint.h>
#include <string.h>
#define SLOTS 65536
#define GROUPS 128
typedef struct { uintptr_t key; uint64_t size; uint32_t group; } Block;
typedef struct { uintptr_t caller; uint64_t allocated, live, calls; } Group;
typedef struct { GMutex lock; uint64_t lost, reallocs, failed_frees; Block blocks[SLOTS]; Group groups[GROUPS]; } State;
extern unsigned char arena[];
#define S ((State *)arena)
typedef struct { uintptr_t old, caller; uint64_t size; Block prior; int nested; } Call;
static Block take(uintptr_t key) {
  Block result = {0};
  if (!key) return result;
  unsigned base = (unsigned)((key >> 4) * 2654435761u) & (SLOTS - 1);
  for (unsigned n = 0; n < 128; n++) {
    Block *b = &S->blocks[(base + n) & (SLOTS - 1)];
    if (b->key == key) { result = *b; b->key = 0; S->groups[b->group].live -= b->size; break; }
  }
  return result;
}
static void put(uintptr_t key, uint64_t size, unsigned group) {
  unsigned base = (unsigned)((key >> 4) * 2654435761u) & (SLOTS - 1);
  for (unsigned n = 0; n < 128; n++) {
    Block *b = &S->blocks[(base + n) & (SLOTS - 1)];
    if (!b->key) { b->key = key; b->size = size; b->group = group; S->groups[group].live += size; return; }
  }
  S->lost++;
}
static void add(uintptr_t key, uint64_t size, uintptr_t caller) {
  if (!key || size < 8192) return;
  for (unsigned g = 0; g < GROUPS; g++) {
    Group *p = &S->groups[g];
    if (!p->caller || p->caller == caller) {
      p->caller = caller; p->allocated += size; p->calls++; put(key, size, g); return;
    }
  }
  S->lost++;
}
void alloc_enter(GumInvocationContext *ic) {
  Call *c = gum_invocation_context_get_listener_invocation_data(ic, sizeof(Call));
  // HeapReAlloc may call HeapAlloc internally. Count only the outer operation.
  c->nested = gum_invocation_context_get_depth(ic) != 0;
  c->size = (uintptr_t)gum_invocation_context_get_nth_argument(ic, 2);
  c->caller = (uintptr_t)gum_invocation_context_get_return_address(ic);
}
void alloc_leave(GumInvocationContext *ic) {
  Call *c = gum_invocation_context_get_listener_invocation_data(ic, sizeof(Call));
  if (c->nested) return;
  g_mutex_lock(&S->lock);
  add((uintptr_t)gum_invocation_context_get_return_value(ic), c->size, c->caller);
  g_mutex_unlock(&S->lock);
}
void free_enter(GumInvocationContext *ic) {
  Call *c = gum_invocation_context_get_listener_invocation_data(ic, sizeof(Call));
  c->nested = gum_invocation_context_get_depth(ic) != 0;
  if (c->nested) return;
  c->old = (uintptr_t)gum_invocation_context_get_nth_argument(ic, 2);
  g_mutex_lock(&S->lock); c->prior = take(c->old); g_mutex_unlock(&S->lock);
}
void free_leave(GumInvocationContext *ic) {
  if ((uintptr_t)gum_invocation_context_get_return_value(ic)) return;
  Call *c = gum_invocation_context_get_listener_invocation_data(ic, sizeof(Call));
  if (c->nested) return;
  g_mutex_lock(&S->lock);
  S->failed_frees++;
  if (c->prior.key) put(c->prior.key, c->prior.size, c->prior.group);
  g_mutex_unlock(&S->lock);
}
void realloc_enter(GumInvocationContext *ic) {
  free_enter(ic);
  Call *c = gum_invocation_context_get_listener_invocation_data(ic, sizeof(Call));
  c->size = (uintptr_t)gum_invocation_context_get_nth_argument(ic, 3);
  c->caller = (uintptr_t)gum_invocation_context_get_return_address(ic);
}
void realloc_leave(GumInvocationContext *ic) {
  Call *c = gum_invocation_context_get_listener_invocation_data(ic, sizeof(Call));
  if (c->nested) return;
  uintptr_t result = (uintptr_t)gum_invocation_context_get_return_value(ic);
  g_mutex_lock(&S->lock);
  S->reallocs++;
  if (result) add(result, c->size, c->caller);
  else if (c->prior.key) put(c->prior.key, c->prior.size, c->prior.group);
  g_mutex_unlock(&S->lock);
}
void snapshot(uint64_t *header, Group *groups) {
  g_mutex_lock(&S->lock);
  header[0] = S->lost; header[1] = S->reallocs; header[2] = S->failed_frees;
  memcpy(groups, S->groups, sizeof(S->groups));
  g_mutex_unlock(&S->lock);
}
`, { arena });
const kernel = Process.getModuleByName('kernel32.dll');
const listeners = [];
for (const [name, prefix] of [['HeapAlloc', 'alloc'], ['HeapFree', 'free'], ['HeapReAlloc', 'realloc']]) {
  listeners.push(Interceptor.attach(kernel.getExportByName(name), {
    onEnter: cm[prefix + '_enter'], onLeave: cm[prefix + '_leave']
  }));
}
const header = Memory.alloc(24), groups = Memory.alloc(128 * 32);
const snapshot = new NativeFunction(cm.snapshot, 'void', ['pointer', 'pointer']);
function dump() {
  snapshot(header, groups);
  const result = [];
  for (let i = 0; i < 128; i++) {
    const row = groups.add(i * 32), caller = row.readPointer();
    if (caller.isNull()) continue;
    const module = Process.findModuleByAddress(caller);
    result.push({caller: module ? module.name + '+' + caller.sub(module.base) : caller.toString(),
      allocated: row.add(8).readU64().toString(), live: row.add(16).readU64().toString(),
      calls: row.add(24).readU64().toString()});
  }
  const resultObject = {type: 'heapTotals', lost: header.readU64().toString(),
    reallocs: header.add(8).readU64().toString(), failedFrees: header.add(16).readU64().toString(), groups: result};
  send(resultObject);
  return resultObject;
}
setInterval(dump, 5000);
rpc.exports = { snapshot: dump };
send({type: 'attached', pid: Process.id, scope: 'HeapAlloc/HeapFree/HeapReAlloc, sizes >=8192 after attachment; no direct Rtl/VirtualAlloc or bulk HeapDestroy accounting; native hooks perturb timing. Snapshots may include in-flight frees/reallocations.'});
