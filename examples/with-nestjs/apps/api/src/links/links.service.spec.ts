import { beforeEach, describe, expect, it } from '@jest/globals';
import { Test, type TestingModule } from '@nestjs/testing';
import { LinksService } from './links.service';

describe('LinksService', () => {
	let service: LinksService;

	beforeEach(async () => {
		const module: TestingModule = await Test.createTestingModule({
			providers: [LinksService],
		}).compile();

		service = module.get<LinksService>(LinksService);
	});

	it('should be defined', () => {
		expect(service).toBeDefined();
	});
});
