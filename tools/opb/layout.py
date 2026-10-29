"""Exact dump selection and explicit consumer bindings, not HDL emitters."""
from . import model


def bindings(profile,items):
    banks={b:[d['id'] for d in items if d['bank']==b] for b in model.BANKS}
    regular=banks['regular64']; payload=banks['payload'][0]; low=banks['low_active1024'][0]
    sels={b:{'sources':ids} for b,ids in banks.items()}
    sels['mixed-xz64']=sels.pop('mixed_xz64'); sels['all-x64']=sels.pop('all_x64')
    sels.update({
      'none':{'sources':[]}, 'all-core':{'sources':[d['id'] for d in items]},
      'repeated64':{'sources':regular[:1],'repeat_to_k':True},
      'active-bit':{'sources':[low],'slices':[[0,0]]},
      'stable-high-slice':{'sources':[low],'slices':[[127,64]]},
      'whole-low-active1024':{'sources':[low]},
      'overlapping64-of1024':{'sources':[payload],'slices':[[i+63,i] for i in range(100)]},
      'concat8x64':{'sources':regular[:8],'operation':'concat-msb-first'},
      'nested16-result-bit':{'sources':regular[16:17]+regular[:16],'operation':'left-fold-concat-msb-first','slice':[0,0]},
      'query-controls-payload':{'sources':banks['drivers']+banks['control']+banks['payload'],
                                'one_driver_sources':banks['drivers'][:1]+banks['control']+banks['payload']},
      'query-shared-base':{'sources':[payload]+banks['control']+[payload], 'slices':[[0,0],None,None],'bases':2},
      'query-distinct-base':{'sources':banks['drivers'][:1]+banks['control']+[payload],'bases':3},
    })
    for alias,target in [('concat8-first','concat8x64'),('concat8-identical-repeat','concat8x64'),('nested16-first','nested16-result-bit')]:
        sels[alias]=dict(sels[target])
    # Both have a local variable and a child scope in the observed tree.
    shallow=['tb_opb','spine']; deep=shallow+['branch','child']*(model.config(profile)['spine_depth']-1)
    return dict(selections=sels,session_variants=[regular,regular[1:]+regular[:1],list(reversed(regular))],
                independent_point=banks['prior'],background_prior=banks['prior'],
                shallow=shallow,deep=deep,missing_last=deep+['missing'],
                concat_orientation='operand 0 is most significant; append 64 bits per level')
