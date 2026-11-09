// Complete own keyword SDK. All objects/targets/AX are synthetic.
#include <ApplicationServices/ApplicationServices.h>
#include <Carbon/Carbon.h>
#include <assert.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <stdint.h>
#include <unistd.h>
#include <limits.h>
typedef struct Obj {unsigned type;uint64_t identity;pid_t pid;UniChar *units;size_t length;CFRange range;AXValueType value_kind;bool valid,enabled,down;uint64_t event_flags;int64_t key,tag,repeat;CGEventTapCallBack callback;void *context;unsigned char *utf8,*html;size_t utf8_length,html_length;bool has_html;} Obj;
static unsigned allocated,app_count,copies,values_read,calls,posts,creates,clipboard_reads,activation_requests,value_unit_reads;
static Obj *tap;
static unsigned char *board_text,*board_html,*paste_text;static size_t board_n,board_h,paste_n;static bool board_has_text,board_has_html,board_extra;static intptr_t board_count;static unsigned board_clears,board_writes,pastes;
static char output[262144];static size_t output_n;
static UniChar *clip,*bundle;static size_t clip_length,bundle_length;static bool custom_bundle;
static unsigned flags;
static pid_t front_pid=42;
static uint64_t app_identity=1,element_identity=1;
static CFRange selection={3,0};
static UniChar *text;static size_t text_length;
static char failure[100],hook_name[100],change_name[100];static unsigned change_flags,change_after=1;static pid_t change_pid;static uint64_t change_app,change_element;static void(*hook)(void*);static void *hook_context;
static int call(const char *name){calls++;bool file_failure=false;
 const char *home=getenv("RUYCAST_KEYWORD_FIXTURE_HOME");if(home){char fault[PATH_MAX];snprintf(fault,sizeof(fault),"%s/keyword-fail-%s",home,name);file_failure=access(fault,F_OK)==0;char hold[PATH_MAX],entered[PATH_MAX],unblock[PATH_MAX];snprintf(hold,sizeof(hold),"%s/hold-keyword-%s",home,name);if(access(hold,F_OK)==0){snprintf(entered,sizeof(entered),"%s/keyword-%s-entered",home,name);FILE *f=fopen(entered,"w");assert(f);fputs("entered",f);fclose(f);snprintf(unblock,sizeof(unblock),"%s/keyword-%s-release",home,name);while(access(unblock,F_OK)!=0)usleep(1000);}}
if(change_name[0]&&strcmp(name,change_name)==0&&--change_after==0){change_name[0]=0;flags=change_flags;front_pid=change_pid;app_identity=change_app;element_identity=change_element;}if(hook&&strcmp(name,hook_name)==0){void(*cb)(void*)=hook;hook=NULL;cb(hook_context);}return file_failure||strcmp(name,failure)==0;}
static Obj *object(unsigned type){Obj *p=calloc(1,sizeof(*p));assert(p);p->type=type;allocated++;return p;}
static Obj *string(const UniChar *units,size_t n){Obj *p=object(2);p->length=n;p->units=malloc((n?n:1)*sizeof(UniChar));assert(p->units);if(n)memcpy(p->units,units,n*sizeof(UniChar));return p;}
static Obj *ascii(const char *s){size_t n=strlen(s);UniChar *u=malloc((n?n:1)*2);assert(u);for(size_t i=0;i<n;i++)u[i]=(unsigned char)s[i];Obj *p=string(u,n);free(u);return p;}
static void release(Obj *p){assert(p&&allocated);if(p==tap){assert(!p->valid);tap=NULL;}free(p->units);free(p->utf8);free(p->html);free(p);allocated--;}
uint64_t ruycast_keyword_fixture_ready(void){return UINT64_C(0x52554b5744303031);}
void ruycast_keyword_fixture_hook(const char *name,void(*cb)(void*),void *context){snprintf(hook_name,sizeof(hook_name),"%s",name);hook=cb;hook_context=context;}
void ruycast_keyword_fixture_failure(const char *name){snprintf(failure,sizeof(failure),"%s",name);}
void ruycast_keyword_fixture_environment(unsigned value,pid_t pid,uint64_t app,uint64_t element){flags=value;front_pid=pid;app_identity=app;element_identity=element;}
void ruycast_keyword_fixture_change(const char *name,unsigned value,pid_t pid,uint64_t app,uint64_t element){snprintf(change_name,sizeof(change_name),"%s",name);change_flags=value;change_pid=pid;change_app=app;change_element=element;change_after=1;}
void ruycast_keyword_fixture_text(const UniChar *units,size_t n,CFIndex at,CFIndex length){free(text);text=malloc((n?n:1)*2);assert(text);if(n)memcpy(text,units,n*2);text_length=n;selection=(CFRange){at,length};}
static void bytes_json(char *out,size_t cap,size_t *at,const unsigned char *data,size_t count){
 if(count>8192){int n=snprintf(out+*at,cap-*at,"null");assert(n>0&&(size_t)n<cap-*at);*at+=(size_t)n;return;}
 assert(*at+count*4+3<cap);out[(*at)++]='[';for(size_t i=0;i<count;i++){int n=snprintf(out+*at,cap-*at,"%s%u",i?",":"",data[i]);assert(n>0);*at+=(size_t)n;}out[(*at)++]=']';out[*at]=0;
}
const char *ruycast_keyword_fixture_state(void){static char out[600000];int n=snprintf(out,sizeof(out),"{\"allocated\":%u,\"apps\":%u,\"copies\":%u,\"valueReads\":%u,\"valueUnitReads\":%u,\"calls\":%u,\"posts\":%u,\"activationRequests\":%u,\"tapCreates\":%u,\"tapValid\":%s,\"tapEnabled\":%s,\"clipboardReads\":%u,\"output\":[%s],\"boardCount\":%ld,\"boardClears\":%u,\"boardWrites\":%u,\"pasteIssues\":%u,\"boardTextBytes\":%zu,\"boardHTMLBytes\":%zu,\"boardText\":",allocated,app_count,copies,values_read,value_unit_reads,calls,posts,activation_requests,creates,tap&&tap->valid?"true":"false",tap&&tap->enabled?"true":"false",clipboard_reads,output,(long)board_count,board_clears,board_writes,pastes,board_n,board_h);assert(n>0);size_t at=(size_t)n;
 bytes_json(out,sizeof(out),&at,board_text,board_n);n=snprintf(out+at,sizeof(out)-at,",\"boardHTML\":");at+=(size_t)n;if(board_has_html)bytes_json(out,sizeof(out),&at,board_html,board_h);else{memcpy(out+at,"null",4);at+=4;}n=snprintf(out+at,sizeof(out)-at,",\"pasteText\":");at+=(size_t)n;bytes_json(out,sizeof(out),&at,paste_text,paste_n);out[at++]='}';out[at]=0;return out;}
void *ruycast_keyword_fixture_front(void){if(call("front")||flags&1)return NULL;Obj *p=object(10);p->pid=front_pid;p->identity=app_identity;app_count++;return p;}
void ruycast_keyword_fixture_app_release(void *app){call("app_release");assert(app_count&&((Obj*)app)->type==10);app_count--;release(app);}
int32_t ruycast_keyword_fixture_app_pid(void *app){call("app_pid");return ((Obj*)app)->pid;}
void *ruycast_keyword_fixture_app_bundle(void *app){assert(((Obj*)app)->type==10);if(call("app_bundle")||flags&2)return NULL;return custom_bundle?string(bundle,bundle_length):ascii("own.keyword.target");}
bool ruycast_keyword_fixture_app_equal(void *a,void *b){call("app_equal");return ((Obj*)a)->identity==((Obj*)b)->identity&&((Obj*)a)->pid==((Obj*)b)->pid;}
bool ruycast_keyword_fixture_match_notification_identity(void *a,uint64_t identity,int32_t pid){assert(((Obj*)a)->type==10);return !call("app_matches_notification")&&((Obj*)a)->identity==identity&&((Obj*)a)->pid==pid;}
bool ruycast_keyword_fixture_app_terminated(void *app){assert(((Obj*)app)->type==10);call("app_terminated");return (flags&4)!=0;}
bool ruycast_keyword_fixture_app_activate(void *app){activation_requests++;assert(((Obj*)app)->type==10);return !call("app_activate");}
void *ruycast_keyword_fixture_attr(uint8_t kind){call("attr");assert(kind<4);return (void*)(uintptr_t)(101+kind);}
void ruycast_keyword_fixture_CFRelease(CFTypeRef value){assert(value&&((Obj*)value)->type!=13);call("release");release((Obj*)value);}
CFTypeID ruycast_keyword_fixture_CFGetTypeID(CFTypeRef value){call("type_id");return ((Obj*)value)->type;}
Boolean ruycast_keyword_fixture_CFEqual(CFTypeRef a,CFTypeRef b){call("equal");return ((Obj*)a)->type==((Obj*)b)->type&&((Obj*)a)->identity==((Obj*)b)->identity;}
CFTypeID ruycast_keyword_fixture_CFStringGetTypeID(void){call("string_type");return 2;}
CFIndex ruycast_keyword_fixture_CFStringGetLength(CFStringRef value){assert(((Obj*)value)->type==2);if(call("string_len"))return -1;if(((Obj*)value)->identity==UINT64_MAX&&strcmp(failure,"value_len_huge")==0)return LONG_MAX/2;return ((Obj*)value)->length;}
void ruycast_keyword_fixture_CFStringGetCharacters(CFStringRef value,CFRange range,UniChar *out){Obj *p=(Obj*)value;assert(p->type==2&&range.location>=0&&range.length>=0&&(size_t)range.location<=p->length&&(size_t)range.length<=p->length-(size_t)range.location&&out);call("string_units");if(p->identity==UINT64_MAX){value_unit_reads++;call("value_units");}if(range.length)memcpy(out,p->units+(size_t)range.location,(size_t)range.length*2);}
AXUIElementRef ruycast_keyword_fixture_AXUIElementCreateSystemWide(void){if(call("system"))return NULL;return (AXUIElementRef)object(11);}
AXUIElementRef ruycast_keyword_fixture_AXUIElementCreateApplication(pid_t pid){assert(pid>0);if(call("ax_app"))return NULL;Obj *p=object(12);p->pid=pid;return (AXUIElementRef)p;}
AXError ruycast_keyword_fixture_AXUIElementSetMessagingTimeout(AXUIElementRef element,float seconds){assert(seconds>0.009f&&seconds<0.011f);unsigned type=((Obj*)element)->type;return call(type==11?"timeout:system":type==12?"timeout:app":"timeout:element")?kAXErrorCannotComplete:kAXErrorSuccess;}
AXError ruycast_keyword_fixture_AXUIElementCopyAttributeValue(AXUIElementRef element,CFStringRef attribute,CFTypeRef *out){assert(out&&!*out);copies++;Obj *p=(Obj*)element;uintptr_t kind=(uintptr_t)attribute-101;assert(kind<4);char name[40];snprintf(name,sizeof(name),"copy:%lu",(unsigned long)kind);bool failed=call(name);Obj *result=NULL;
    if(kind==0){assert(p->type==11||p->type==12);if(!(flags&32)){result=object(flags&64?4:1);result->identity=element_identity;}}
    else{assert(p->type==1);if(kind==1&&!(flags&128))result=ascii(flags&16?"AXSecureTextField":"AXTextField");
        if(kind==2&&!(flags&256)){result=object(3);result->value_kind=flags&512?kAXValueTypeCGPoint:kAXValueTypeCFRange;result->range=selection;}
        if(kind==3){values_read++;if(!(flags&1024)){result=string(text,text_length);result->identity=UINT64_MAX;}}}
    if(failed&&!(flags&2048)){if(result)release(result);result=NULL;}
    *out=result;return failed?kAXErrorCannotComplete:kAXErrorSuccess;
}
CFTypeID ruycast_keyword_fixture_AXUIElementGetTypeID(void){call("ax_type");return 1;}
#ifndef OMIT_KEYWORD_VALUE_TYPE
CFTypeID ruycast_keyword_fixture_AXValueGetTypeID(void){call("value_type");return 3;}
#endif
AXValueType ruycast_keyword_fixture_AXValueGetType(AXValueRef value){assert(((Obj*)value)->type==3);call("value_kind");return ((Obj*)value)->value_kind;}
Boolean ruycast_keyword_fixture_AXValueGetValue(AXValueRef value,AXValueType kind,void *out){assert(((Obj*)value)->type==3&&kind==kAXValueTypeCFRange&&out);if(call("value_range"))return false;*(CFRange*)out=((Obj*)value)->range;return true;}
Boolean ruycast_keyword_fixture_IsSecureEventInputEnabled(void){call("secure");return (flags&8)!=0;}
Boolean ruycast_keyword_fixture_AXIsProcessTrusted(void){call("trusted");return (flags&4096)==0;}
#define ABI(name) _Static_assert(__builtin_types_compatible_p(__typeof__(&(name)),__typeof__(&(ruycast_keyword_fixture_##name))),#name " ABI")
ABI(CFRelease);ABI(CFGetTypeID);ABI(CFEqual);ABI(CFStringGetTypeID);ABI(CFStringGetLength);ABI(CFStringGetCharacters);ABI(AXUIElementCreateSystemWide);ABI(AXUIElementCreateApplication);ABI(AXUIElementSetMessagingTimeout);ABI(AXUIElementCopyAttributeValue);ABI(AXUIElementGetTypeID);
#ifndef OMIT_KEYWORD_VALUE_TYPE
ABI(AXValueGetTypeID);
#endif
ABI(AXValueGetType);ABI(AXValueGetValue);ABI(IsSecureEventInputEnabled);ABI(AXIsProcessTrusted);_Static_assert(sizeof(Boolean)==1&&sizeof(CFRange)==16&&kAXValueTypeCFRange==4,"keyword SDK scalars/constants");

void *ruycast_keyword_fixture_common(void){call("common");return (void*)(uintptr_t)200;}
bool ruycast_keyword_fixture_raycast(void){call("raycast");return (flags&8192)!=0;}
void *ruycast_keyword_fixture_clipboard(void){clipboard_reads++;if(call("clipboard"))return NULL;return string(clip,clip_length);}
void ruycast_keyword_fixture_set_clipboard(const UniChar *u,size_t n){board_count++;board_has_text=true;board_has_html=false;board_extra=true;free(clip);clip=malloc((n?n:1)*2);assert(clip);if(n)memcpy(clip,u,n*2);clip_length=n;}
void ruycast_keyword_fixture_clear_output(void){output_n=0;output[0]=0;posts=0;pastes=0;free(paste_text);paste_text=NULL;paste_n=0;}
CFMachPortRef ruycast_keyword_fixture_CGEventTapCreate(CGEventTapLocation location,CGEventTapPlacement place,CGEventTapOptions options,CGEventMask mask,CGEventTapCallBack cb,void *context){assert(location==kCGSessionEventTap&&place==kCGHeadInsertEventTap&&options==kCGEventTapOptionDefault&&mask==((UINT64_C(1)<<1)|(UINT64_C(1)<<3)|(UINT64_C(1)<<10)));if(call("tap_create"))return NULL;assert(!tap);tap=object(8);tap->valid=true;tap->callback=cb;tap->context=context;creates++;return (CFMachPortRef)tap;}
void ruycast_keyword_fixture_CGEventTapEnable(CFMachPortRef port,bool enabled){assert((Obj*)port==tap);if(!call(enabled?"tap_enable":"tap_disable"))tap->enabled=enabled;}
bool ruycast_keyword_fixture_CGEventTapIsEnabled(CFMachPortRef port){assert((Obj*)port==tap);return !call("tap_enabled")&&tap->enabled;}
void ruycast_keyword_fixture_CFMachPortInvalidate(CFMachPortRef port){assert((Obj*)port==tap);if(!call("invalidate"))tap->valid=false;}
Boolean ruycast_keyword_fixture_CFMachPortIsValid(CFMachPortRef port){assert((Obj*)port==tap);call("valid");return tap->valid;}
CFRunLoopSourceRef ruycast_keyword_fixture_CFMachPortCreateRunLoopSource(CFAllocatorRef allocator,CFMachPortRef port,CFIndex order){assert(!allocator&&(Obj*)port==tap&&order==0);return call("source")?NULL:(CFRunLoopSourceRef)object(9);}
CFRunLoopRef ruycast_keyword_fixture_CFRunLoopGetMain(void){call("runloop");return (CFRunLoopRef)(uintptr_t)201;}
void ruycast_keyword_fixture_CFRunLoopAddSource(CFRunLoopRef loop,CFRunLoopSourceRef source,CFRunLoopMode mode){assert((uintptr_t)loop==201&&((Obj*)source)->type==9&&(uintptr_t)mode==200);call("add_source");}
void ruycast_keyword_fixture_CFRunLoopRemoveSource(CFRunLoopRef loop,CFRunLoopSourceRef source,CFRunLoopMode mode){assert((uintptr_t)loop==201&&((Obj*)source)->type==9&&(uintptr_t)mode==200);call("remove_source");}
int64_t ruycast_keyword_fixture_CGEventGetIntegerValueField(CGEventRef value,CGEventField field){call("field");Obj *p=(Obj*)value;if(field==kCGEventSourceUserData)return p->tag;if(field==kCGKeyboardEventKeycode)return p->key;assert(field==kCGKeyboardEventAutorepeat);return p->repeat;}
CGEventFlags ruycast_keyword_fixture_CGEventGetFlags(CGEventRef value){call("flags");return ((Obj*)value)->event_flags;}
void ruycast_keyword_fixture_CGEventKeyboardGetUnicodeString(CGEventRef value,UniCharCount max,UniCharCount *actual,UniChar *out){call("unicode");Obj *p=(Obj*)value;assert(max==16&&actual&&out);*actual=p->length;size_t n=p->length<max?p->length:max;if(n)memcpy(out,p->units,n*2);}
CGEventRef ruycast_keyword_fixture_CGEventCreateKeyboardEvent(CGEventSourceRef source,CGKeyCode key,bool down){assert(!source);if(call("keyboard_event")||call(down?"event:down":"event:up"))return NULL;Obj *p=object(5);p->key=key;p->down=down;if(key==9){p->units=malloc(2);assert(p->units);p->units[0]=118;p->length=1;}return (CGEventRef)p;}
void ruycast_keyword_fixture_CGEventSetFlags(CGEventRef value,CGEventFlags mask){((Obj*)value)->event_flags=mask;call("set_flags");}
void ruycast_keyword_fixture_CGEventSetIntegerValueField(CGEventRef value,CGEventField field,int64_t data){assert(field==kCGEventSourceUserData);((Obj*)value)->tag=data;call("set_field");}
void ruycast_keyword_fixture_CGEventKeyboardSetUnicodeString(CGEventRef value,UniCharCount count,const UniChar *units){Obj *p=(Obj*)value;free(p->units);p->units=malloc((count?count:1)*2);assert(p->units);if(count)memcpy(p->units,units,count*2);p->length=count;call("set_unicode");}
void ruycast_keyword_fixture_CGEventPost(CGEventTapLocation location,CGEventRef value){Obj *p=(Obj*)value;assert(location==kCGHIDEventTap&&p->type==5&&p->tag==INT64_C(0x5255535443415354));posts++;
 if(p->key==9&&p->down&&(p->event_flags&UINT64_C(1)<<20)){free(paste_text);paste_text=malloc(board_n?board_n:1);assert(paste_text);if(board_n)memcpy(paste_text,board_text,board_n);paste_n=board_n;pastes++;}
 if(p->key==123&&p->down){const char *home=getenv("RUYCAST_KEYWORD_FIXTURE_HOME");if(home){char path[PATH_MAX];snprintf(path,sizeof(path),"%s/keyword-cursor-entered",home);FILE *f=fopen(path,"w");assert(f);fputs("issued",f);fclose(f);}}
 if(output_n+100+p->length*8<sizeof(output)){int n=snprintf(output+output_n,sizeof(output)-output_n,"%s{\"key\":%lld,\"down\":%s,\"flags\":%llu,\"units\":[",output_n?",":"",(long long)p->key,p->down?"true":"false",(unsigned long long)p->event_flags);assert(n>0);output_n+=(size_t)n;for(size_t i=0;i<p->length;i++){n=snprintf(output+output_n,sizeof(output)-output_n,"%s%u",i?",":"",p->units[i]);assert(n>0);output_n+=(size_t)n;}n=snprintf(output+output_n,sizeof(output)-output_n,"]}");output_n+=(size_t)n;}
 call(p->down?"post:down":"post:up");call("post");}
bool ruycast_keyword_fixture_emit(uint32_t kind,int64_t key,uint64_t event_flags,int64_t repeat,int64_t tag,const UniChar *units,size_t count){Obj *p=string(units,count);p->type=5;p->key=key;p->event_flags=event_flags;p->repeat=repeat;p->tag=tag;bool consumed=false;if(tap&&tap->valid&&tap->enabled){CGEventTapCallBack cb=tap->callback;void *context=tap->context;consumed=cb((CGEventTapProxy)(uintptr_t)205,kind,(CGEventRef)p,context)==NULL;}release(p);return consumed;}
ABI(CGEventTapCreate);ABI(CGEventTapEnable);ABI(CGEventTapIsEnabled);ABI(CFMachPortInvalidate);ABI(CFMachPortIsValid);ABI(CFMachPortCreateRunLoopSource);ABI(CFRunLoopGetMain);ABI(CFRunLoopAddSource);ABI(CFRunLoopRemoveSource);ABI(CGEventGetIntegerValueField);ABI(CGEventGetFlags);ABI(CGEventKeyboardGetUnicodeString);ABI(CGEventCreateKeyboardEvent);ABI(CGEventSetFlags);ABI(CGEventSetIntegerValueField);ABI(CGEventKeyboardSetUnicodeString);ABI(CGEventPost);
_Static_assert(sizeof(UniCharCount)==sizeof(size_t)&&kCGHIDEventTap==0,"keyword event scalar ABI");

void ruycast_keyword_fixture_set_bundle(const UniChar *u,size_t n){free(bundle);bundle=malloc((n?n:1)*2);assert(bundle);if(n)memcpy(bundle,u,n*2);bundle_length=n;custom_bundle=true;}

void ruycast_keyword_fixture_change_after(const char *name,unsigned nth,unsigned value,pid_t pid,uint64_t app,uint64_t element){ruycast_keyword_fixture_change(name,value,pid,app,element);assert(nth>0);change_after=nth;}

static unsigned char *duplicate(const unsigned char *p,size_t n){unsigned char *out=malloc(n?n:1);assert(out);if(n)memcpy(out,p,n);return out;}
static void clip_utf8(const unsigned char *p,size_t n){free(clip);clip=malloc((n+1)*2);assert(clip);size_t i=0,j=0;while(i<n){uint32_t scalar=p[i++];if(scalar>=0xc0){unsigned more=scalar<0xe0?1:scalar<0xf0?2:3;scalar&=more==1?31:more==2?15:7;for(unsigned k=0;k<more;k++){assert(i<n);scalar=(scalar<<6)|(p[i++]&63);}}if(scalar<=0xffff)clip[j++]=(UniChar)scalar;else{scalar-=0x10000;clip[j++]=(UniChar)(0xd800+(scalar>>10));clip[j++]=(UniChar)(0xdc00+(scalar&1023));}}clip_length=j;}
static void replace_board(const unsigned char *p,size_t n,const unsigned char *html,size_t h,bool has_html,bool extra){free(board_text);free(board_html);board_text=duplicate(p,n);board_n=n;board_html=duplicate(html,has_html?h:0);board_h=has_html?h:0;board_has_html=has_html;board_has_text=true;board_extra=extra;board_count++;clip_utf8(p,n);}
void *ruycast_keyword_fixture_item_create(const unsigned char *text,size_t n,const unsigned char *html,size_t h,bool has_html){if(call("item_create")||call("item_text"))return NULL;Obj *item=object(13);item->utf8=duplicate(text,n);item->utf8_length=n;item->has_html=has_html;if(has_html){if(call("item_html")){release(item);return NULL;}item->html=duplicate(html,h);item->html_length=h;}return item;}
void ruycast_keyword_fixture_item_release(void *value){assert(value&&((Obj*)value)->type==13);call("item_release");release(value);}
intptr_t ruycast_keyword_fixture_board_clear(void){if(call("board_clear"))return board_count;free(board_text);free(board_html);board_text=board_html=NULL;board_n=board_h=0;board_has_text=board_has_html=board_extra=false;free(clip);clip=NULL;clip_length=0;board_count++;board_clears++;intptr_t own=board_count;call("board_cleared");return own;}
bool ruycast_keyword_fixture_board_write(void *value){Obj *item=value;assert(item&&item->type==13);if(call("board_write"))return false;bool partial=call("board_partial");replace_board(item->utf8,item->utf8_length,item->html,item->html_length,partial?false:item->has_html,false);board_writes++;call("board_written");return !partial;}
intptr_t ruycast_keyword_fixture_board_change(void){intptr_t snapshot=board_count;call("board_change");return snapshot;}
static bool equal_bytes(const unsigned char *a,size_t n,const unsigned char *b,size_t m){return n==m&&(!n||memcmp(a,b,n)==0);}
static bool ours(Obj *item,bool partial){return board_has_text&&!board_extra&&equal_bytes(board_text,board_n,item->utf8,item->utf8_length)&&(partial?(!board_has_html||(item->has_html&&equal_bytes(board_html,board_h,item->html,item->html_length))):(board_has_html==item->has_html&&(!board_has_html||equal_bytes(board_html,board_h,item->html,item->html_length))));}
bool ruycast_keyword_fixture_board_matches(void *value){Obj *item=value;assert(item&&item->type==13);bool answer=ours(item,false);return !call("board_matches")&&answer;}
bool ruycast_keyword_fixture_board_ours(void *value){Obj *item=value;assert(item&&item->type==13);bool answer=ours(item,true);return !call("board_ours")&&answer;}
void ruycast_keyword_fixture_board_external(const unsigned char *text,size_t n,const unsigned char *html,size_t h,bool has_html,bool extra){replace_board(text,n,html,h,has_html,extra);}
