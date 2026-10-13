#include "fstapi.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void fail(const char *message) { fprintf(stderr,"writer: %s\n",message); exit(1); }
static void read_bytes(FILE *input,void *output,size_t n) { if(fread(output,1,n,input)!=n) fail("short input"); }
static uint64_t number(FILE *input,unsigned n) { unsigned char bytes[8]; uint64_t value=0; read_bytes(input,bytes,n); for(unsigned i=0;i<n;i++) value|=(uint64_t)bytes[i]<<(8*i); return value; }

int main(int argc,char **argv) {
    if(argc!=3) fail("STREAM OUTPUT.fst");
    FILE *input=fopen(argv[1],"rb"); if(!input) fail("open input");
    char magic[8]; read_bytes(input,magic,8); if(memcmp(magic,"OWAV0001",8)) fail("magic");
    uint32_t n=number(input,4); uint64_t first=number(input,8),last=number(input,8);
    fstHandle *handles=calloc(n,sizeof(*handles));
    uint32_t *widths=calloc(n,sizeof(*widths)); unsigned char *kinds=calloc(n,1);
    if(!handles||!widths||!kinds||first>last) fail("header");
    fstWriterContext *writer=fstWriterCreate(argv[2],1); if(!writer) fail("create writer");
    fstWriterSetPackType(writer,FST_WR_PT_LZ4);
    fstWriterSetTimescale(writer,-15);
    fstWriterSetVersion(writer,"Ondas normalized-history comparison");
    fstWriterSetScope(writer,FST_ST_VCD_MODULE,"export",NULL);
    for(uint32_t i=0;i<n;i++) {
        kinds[i]=number(input,1); widths[i]=number(input,4);
        if(kinds[i]>1||!widths[i]||widths[i]>1048576) fail("signal geometry");
        char name[32]; snprintf(name,sizeof(name),"s%u",i);
        handles[i]=fstWriterCreateVar(writer,kinds[i]?FST_VT_VCD_REAL:FST_VT_VCD_WIRE,FST_VD_IMPLICIT,widths[i],name,0);
        if(!handles[i]) fail("create variable");
    }
    fstWriterSetUpscope(writer); fstWriterEmitTimeChange(writer,first);
    uint64_t current=first,rows=0;
    unsigned char *value=NULL; size_t capacity=0;
    for(;;) {
        uint64_t time=number(input,8); uint32_t index=number(input,4),len=number(input,4);
        if(time<current||time>last) fail("time order");
        if(time!=current) { fstWriterEmitTimeChange(writer,time); current=time; }
        if(index==UINT32_MAX) { if(len||time!=last) fail("end marker"); break; }
        if(index>=n||len!=(kinds[index]?8:widths[index])) fail("value geometry");
        if((size_t)len+1>capacity) { capacity=(size_t)len+1; value=realloc(value,capacity); if(!value) fail("allocation"); }
        read_bytes(input,value,len); value[len]=0;
        if(kinds[index]) { uint64_t bits=0; for(unsigned i=0;i<8;i++) bits|=(uint64_t)value[i]<<(8*i); double real; memcpy(&real,&bits,8); fstWriterEmitValueChange(writer,handles[index],&real); }
        else { for(uint32_t i=0;i<len;i++) if(!strchr("01xzhuwl-",value[i])) fail("logic byte"); fstWriterEmitValueChange(writer,handles[index],value); }
        rows++;
    }
    if(fgetc(input)!=EOF) fail("trailing bytes");
    fclose(input); fstWriterClose(writer);
    printf("wrote signals=%u rows=%" PRIu64 " first=%" PRIu64 " last=%" PRIu64 "\n",n,rows,first,last);
    free(value); free(handles); free(widths); free(kinds);
    return 0;
}
