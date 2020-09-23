# Includes
include_directories( ${CMAKE_CURRENT_SOURCE_DIR}
		${CMAKE_CURRENT_BINARY_DIR}
		)

# Targets
add_executable(simple simple.c)
add_executable(bitbang bitbang.c)
add_executable(bitbang2 bitbang2.c)
add_executable(bitbang_cbus bitbang_cbus.c)
add_executable(bitbang_ft2232 bitbang_ft2232.c)
add_executable(find_all find_all.c)
add_executable(serial_test serial_test.c)
add_executable(baud_test baud_test.c)
add_executable(stream_test stream_test.c)
add_executable(eeprom eeprom.c)
add_executable(async async.c)
add_executable(purge_test purge_test.c)

# Linkage
target_link_libraries(simple ftdi1)
target_link_libraries(bitbang ftdi1)
target_link_libraries(bitbang2 ftdi1)
target_link_libraries(bitbang_cbus ftdi1)
target_link_libraries(bitbang_ft2232 ftdi1)
target_link_libraries(find_all ftdi1)
target_link_libraries(serial_test ftdi1)
target_link_libraries(baud_test ftdi1)
target_link_libraries(stream_test ftdi1)
target_link_libraries(eeprom ftdi1)
target_link_libraries(async ftdi1)
target_link_libraries(purge_test ftdi1)

# libftdi++ examples
if( FTDIPP )
	include_directories(BEFORE ${CMAKE_SOURCE_DIR}/ftdipp
			${Boost_INCLUDE_DIRS})

	# Target
	add_executable(find_all_pp find_all_pp.cpp)

	# Linkage
	target_link_libraries(find_all_pp ftdipp1)
endif( FTDIPP )

# Source includes
include_directories(BEFORE ${CMAKE_SOURCE_DIR}/src)
