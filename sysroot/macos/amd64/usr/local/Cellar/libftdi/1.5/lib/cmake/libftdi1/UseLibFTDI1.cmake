#                                               -*- cmake -*-
#
#  UseLibFTDI.cmake
#
#  Copyright (C) 2013 Intra2net AG and the libftdi developers
#
#  This file is part of LibFTDI.
#
#  LibFTDI is free software; you can redistribute it and/or modify
#  it under the terms of the GNU Lesser General Public License
#  version 2.1 as published by the Free Software Foundation;
#


add_definitions     ( ${LIBFTDI_DEFINITIONS} )
include_directories ( ${LIBFTDI_INCLUDE_DIRS} )
link_directories    ( ${LIBFTDI_LIBRARY_DIRS} )

