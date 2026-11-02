import sys

if sys.argv[1:2] == ['view']:
    from .viewer import main
    raise SystemExit(main(sys.argv[2:]))

from .runner import main

raise SystemExit(main())
